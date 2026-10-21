//! The term language, what each e-class knows, and the rewrite rules.
//!
//! Columns are `alias.column` symbols, so a rule never has to renumber
//! anything. Every expression rule is an equality of SQL's three-valued logic
//! (it holds for TRUE, FALSE and UNKNOWN alike), so it is sound anywhere, not
//! only at the top of a WHERE. Rules that only hold where a predicate decides
//! which rows pass (`IN` against `EXISTS`) rewrite whole `filter` nodes.

use std::collections::{BTreeMap, BTreeSet};

use egg::{
    define_language, rewrite as rw, Analysis, Applier, DidMerge, EGraph, Id, Language, PatternAst, Rewrite, Subst,
    Symbol, Var,
};

define_language! {
    pub enum Sql {
        // (project rel col...)
        "project" = Project(Box<[Id]>),
        // SELECT DISTINCT over a project.
        "distinct" = Distinct([Id; 1]),
        // (group-by rel col...): SELECT col... FROM rel GROUP BY col..., no aggregates.
        "group-by" = GroupBy(Box<[Id]>),
        // (filter predicate rel)
        "filter" = Filter([Id; 2]),
        // A cross product; an inner join is a filter over one.
        "join" = Join([Id; 2]),
        // (left-join on left right)
        "left-join" = LeftJoin([Id; 3]),
        // (scan table alias)
        "scan" = Scan([Id; 2]),
        "and" = And([Id; 2]),
        "or" = Or([Id; 2]),
        "not" = Not([Id; 1]),
        "=" = Eq([Id; 2]),
        "<>" = Ne([Id; 2]),
        "<" = Lt([Id; 2]),
        "<=" = Le([Id; 2]),
        ">" = Gt([Id; 2]),
        ">=" = Ge([Id; 2]),
        "+" = Add([Id; 2]),
        "is-null" = IsNull([Id; 1]),
        "between" = Between([Id; 3]),
        // (in needle item...)
        "in" = In(Box<[Id]>),
        // (in-sub needle (project rel col)): `needle IN (SELECT col FROM rel)`.
        "in-sub" = InSub([Id; 2]),
        // (exists rel): `EXISTS (SELECT * FROM rel)`, usually correlated.
        "exists" = Exists([Id; 1]),
        // `alias.column`, literals, TRUE, FALSE, NULL, table names and aliases.
        Symbol(Symbol),
    }
}

/// What the schema says, by alias.
#[derive(Default)]
pub struct Facts {
    /// `alias.column` for NOT NULL columns, except on tables that are ever on
    /// the null-supplied side of a LEFT JOIN: their padded rows are NULL.
    pub not_null: BTreeSet<String>,
    /// Single-column keys (PRIMARY KEY, or UNIQUE and NOT NULL) by alias.
    pub keys: BTreeMap<String, Vec<String>>,
}

/// What an e-class knows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Data {
    /// A relation's aliases: the tables whose columns it outputs.
    pub provided: BTreeSet<Symbol>,
    /// Aliases an expression reads, or that a relation reads from outside
    /// itself (a correlated subquery).
    pub reads: BTreeSet<Symbol>,
    /// Aliases whose columns all being NULL makes this expression NULL.
    pub strict: BTreeSet<Symbol>,
    /// Aliases whose columns all being NULL make this predicate not TRUE:
    /// it rejects the rows a LEFT JOIN pads.
    pub rejects: BTreeSet<Symbol>,
    pub symbol: Option<Symbol>,
}

/// `u.name` is a column of `u`; `1.5` and `'a.b'` are literals.
pub fn column_alias(s: &str) -> Option<&str> {
    let first = s.chars().next()?;
    (first.is_alphabetic() || first == '_').then(|| s.split_once('.').map(|(a, _)| a)).flatten()
}

fn union<'a>(sets: impl IntoIterator<Item = &'a BTreeSet<Symbol>>) -> BTreeSet<Symbol> {
    sets.into_iter().flatten().copied().collect()
}

impl Analysis<Sql> for Facts {
    type Data = Data;

    fn make(egraph: &mut EGraph<Sql, Self>, enode: &Sql, _id: Id) -> Data {
        let d = |id: &Id| &egraph[*id].data;
        let reads = || union(enode.children().iter().map(|c| &d(c).reads));
        let strict = |ids: &[Id]| union(ids.iter().map(|c| &d(c).strict));
        let mut data = Data::default();
        match enode {
            Sql::Symbol(s) => {
                if let Some(a) = column_alias(s.as_str()) {
                    let a: BTreeSet<Symbol> = [Symbol::from(a)].into();
                    (data.reads, data.strict, data.rejects) = (a.clone(), a.clone(), a);
                }
                data.symbol = Some(*s);
            },
            Sql::Scan([_, alias]) => data.provided = d(alias).symbol.into_iter().collect(),
            Sql::Join(c) => (data.provided, data.reads) = (union(c.iter().map(|c| &d(c).provided)), reads()),
            Sql::LeftJoin([_, a, b]) | Sql::Filter([a, b]) => {
                data.provided = if matches!(enode, Sql::LeftJoin(_)) {
                    union([&d(a).provided, &d(b).provided])
                } else {
                    d(b).provided.clone()
                };
                data.reads = &reads() - &data.provided;
            },
            Sql::Project(c) | Sql::GroupBy(c) => {
                data.provided = d(&c[0]).provided.clone();
                data.reads = &reads() - &data.provided;
            },
            Sql::Distinct([p]) => (data.provided, data.reads) = (d(p).provided.clone(), d(p).reads.clone()),
            Sql::Exists([r]) => data.reads = d(r).reads.clone(),
            Sql::InSub([x, _]) => (data.reads, data.rejects) = (reads(), d(x).strict.clone()),
            Sql::Add(c) => (data.reads, data.strict) = (reads(), strict(c)),
            Sql::Eq(c) | Sql::Ne(c) | Sql::Lt(c) | Sql::Le(c) | Sql::Gt(c) | Sql::Ge(c) => {
                (data.reads, data.rejects) = (reads(), strict(c))
            },
            Sql::Between(c) => (data.reads, data.rejects) = (reads(), strict(c)),
            Sql::In(c) => (data.reads, data.rejects) = (reads(), d(&c[0]).strict.clone()),
            Sql::And([a, b]) => (data.reads, data.rejects) = (reads(), union([&d(a).rejects, &d(b).rejects])),
            Sql::Or([a, b]) => (data.reads, data.rejects) = (reads(), &d(a).rejects & &d(b).rejects),
            // `NOT (x IS NULL)` is FALSE when `x` is NULL; any other NOT, nothing known.
            Sql::Not([a]) => {
                data.reads = reads();
                let is_null_of = egraph[*a].nodes.iter().find_map(|n| match n {
                    Sql::IsNull([x]) => Some(*x),
                    _ => None,
                });
                if let Some(x) = is_null_of {
                    data.rejects = d(&x).strict.clone();
                }
            },
            Sql::IsNull(_) => data.reads = reads(),
        }
        data
    }

    /// Equivalent classes have the same meaning, so a fact either knows is
    /// true of both: `strict` and `rejects` merge by union. `reads` merges by
    /// union and `provided` by intersection, the conservative direction for
    /// the conditions that read them (should equivalent classes ever differ).
    fn merge(&mut self, a: &mut Data, b: Data) -> DidMerge {
        let mut merged = a.clone();
        merged.reads.extend(b.reads.iter().copied());
        merged.strict.extend(b.strict.iter().copied());
        merged.rejects.extend(b.rejects.iter().copied());
        merged.provided = &a.provided & &b.provided;
        merged.symbol = a.symbol.or(b.symbol);
        let changed = (merged != *a, merged != b);
        *a = merged;
        DidMerge(changed.0, changed.1)
    }
}

type Condition = Box<dyn Fn(&mut EGraph<Sql, Facts>, Id, &Subst) -> bool + Send + Sync>;

fn var(v: &str) -> Var {
    v.parse().unwrap()
}

/// `p` reads only aliases that `rel` provides.
fn reads_only(p: &str, rel: &str) -> Condition {
    let (p, rel) = (var(p), var(rel));
    Box::new(move |eg, _, s| eg[s[p]].data.reads.is_subset(&eg[s[rel]].data.provided))
}

/// `p` reads only aliases that `a` or `b` provides.
fn reads_only_either(p: &str, a: &str, b: &str) -> Condition {
    let (p, a, b) = (var(p), var(a), var(b));
    Box::new(move |eg, _, s| {
        let both = union([&eg[s[a]].data.provided, &eg[s[b]].data.provided]);
        eg[s[p]].data.reads.is_subset(&both)
    })
}

/// `p` reads nothing that `rel` provides.
fn reads_none_of(p: &str, rel: &str) -> Condition {
    let (p, rel) = (var(p), var(rel));
    Box::new(move |eg, _, s| eg[s[p]].data.reads.is_disjoint(&eg[s[rel]].data.provided))
}

/// `p` rejects the rows a LEFT JOIN pads with NULLs for `rel`.
fn rejects_padding_of(p: &str, rel: &str) -> Condition {
    let (p, rel) = (var(p), var(rel));
    Box::new(move |eg, _, s| !eg[s[p]].data.rejects.is_disjoint(&eg[s[rel]].data.provided))
}

/// `rel` reads nothing from outside itself: an uncorrelated subquery.
fn uncorrelated(rel: &str) -> Condition {
    let rel = var(rel);
    Box::new(move |eg, _, s| eg[s[rel]].data.reads.is_empty())
}

/// `c` is a column the schema declares NOT NULL.
fn declared_not_null(c: &str) -> Condition {
    let c = var(c);
    Box::new(move |eg, _, s| eg[s[c]].data.symbol.is_some_and(|sym| eg.analysis.not_null.contains(sym.as_str())))
}

/// `c` is a single-column key of the one table `rel` provides.
fn key_of(c: &str, rel: &str) -> Condition {
    let (c, rel) = (var(c), var(rel));
    Box::new(move |eg, _, s| {
        let Some((alias, col)) = eg[s[c]].data.symbol.and_then(|sym| {
            let (a, c) = sym.as_str().split_once('.')?;
            Some((a.to_string(), c.to_string()))
        }) else {
            return false;
        };
        let provided = &eg[s[rel]].data.provided;
        provided.len() == 1
            && provided.contains(&Symbol::from(alias.as_str()))
            && eg.analysis.keys.get(&alias).is_some_and(|keys| keys.contains(&col))
    })
}

fn all(conditions: Vec<Condition>) -> Condition {
    Box::new(move |eg, id, s| conditions.iter().all(|c| c(eg, id, s)))
}

/// `?p` (a projection) outputs a key of every table it reads from, so its
/// rows are distinct already.
fn rows_distinct(p: &str) -> Condition {
    let p = var(p);
    Box::new(move |eg, _, s| {
        let projects: Vec<Box<[Id]>> = (eg[s[p]].nodes.iter())
            .filter_map(|n| if let Sql::Project(c) = n { Some(c.clone()) } else { None })
            .collect();
        projects.iter().any(|c| {
            let columns: BTreeSet<&str> = c[1..].iter().filter_map(|&k| eg[k].data.symbol.map(|s| s.as_str())).collect();
            let provided = &eg[c[0]].data.provided;
            !provided.is_empty()
                && provided.iter().all(|alias| {
                    eg.analysis.keys.get(alias.as_str()).is_some_and(|keys| {
                        keys.iter().any(|k| columns.contains(format!("{alias}.{k}").as_str()))
                    })
                })
        })
    })
}

/// `SELECT DISTINCT cols` is `SELECT cols ... GROUP BY cols`: puts a
/// `group-by` with the projection's children into the `distinct`'s class.
/// Written by hand because a projection's arity varies.
struct DistinctAsGroupBy(Var);

impl Applier<Sql, Facts> for DistinctAsGroupBy {
    fn apply_one(
        &self,
        egraph: &mut EGraph<Sql, Facts>,
        eclass: Id,
        subst: &Subst,
        _: Option<&PatternAst<Sql>>,
        _: Symbol,
    ) -> Vec<Id> {
        let projects: Vec<Box<[Id]>> = (egraph[subst[self.0]].nodes.iter())
            .filter_map(|n| if let Sql::Project(c) = n { Some(c.clone()) } else { None })
            .collect();
        let mut changed = Vec::new();
        for c in projects {
            let grouped = egraph.add(Sql::GroupBy(c));
            if egraph.union(eclass, grouped) {
                changed.push(eclass);
            }
        }
        changed
    }
}

pub fn rules() -> Vec<Rewrite<Sql, Facts>> {
    let mut rules = vec![
        // Expressions.
        rw!("and-comm"; "(and ?a ?b)" => "(and ?b ?a)"),
        rw!("or-comm"; "(or ?a ?b)" => "(or ?b ?a)"),
        rw!("eq-comm"; "(= ?a ?b)" => "(= ?b ?a)"),
        rw!("ne-comm"; "(<> ?a ?b)" => "(<> ?b ?a)"),
        rw!("not-not"; "(not (not ?a))" => "?a"),
        rw!("not-false"; "(not FALSE)" => "TRUE"),
        rw!("and-true"; "(and ?a TRUE)" => "?a"),
        // The schema: IS NULL of a NOT NULL column (never padded) is FALSE.
        rw!("not-null"; "(is-null ?c)" => "FALSE" if declared_not_null("?c")),
        // Inner joins.
        rw!("join-comm"; "(join ?a ?b)" => "(join ?b ?a)"),
        rw!("filter-up"; "(join (filter ?p ?a) ?b)" => "(filter ?p (join ?a ?b))"),
        rw!("filter-down"; "(filter ?p (join ?a ?b))" => "(join (filter ?p ?a) ?b)" if reads_only("?p", "?a")),
        // LEFT JOIN. A filter on the preserved side commutes with the join; a
        // conjunct of ON that reads only the null-supplied side filters it
        // first; a filter above that rejects the padded rows makes it inner.
        rw!("left-filter-down"; "(filter ?p (left-join ?q ?a ?b))" => "(left-join ?q (filter ?p ?a) ?b)"
            if reads_only("?p", "?a")),
        rw!("left-filter-up"; "(left-join ?q (filter ?p ?a) ?b)" => "(filter ?p (left-join ?q ?a ?b))"),
        rw!("left-on-right"; "(left-join (and ?q ?r) ?a ?b)" => "(left-join ?q ?a (filter ?r ?b))"
            if reads_only("?r", "?b")),
        rw!("left-on-right-rev"; "(left-join ?q ?a (filter ?r ?b))" => "(left-join (and ?q ?r) ?a ?b)"),
        rw!("left-to-inner"; "(filter ?p (left-join ?q ?a ?b))" => "(filter ?p (filter ?q (join ?a ?b)))"
            if rejects_padding_of("?p", "?b")),
        // (A ⟕p B) ⟕q C = A ⟕p (B ⟕q C) when q reads only B and C and rejects B's padding.
        rw!("left-assoc"; "(left-join ?q (left-join ?p ?a ?b) ?c)" => "(left-join ?p ?a (left-join ?q ?b ?c))"
            if all(vec![reads_only_either("?q", "?b", "?c"), rejects_padding_of("?q", "?b")])),
        rw!("left-assoc-rev"; "(left-join ?p ?a (left-join ?q ?b ?c))" => "(left-join ?q (left-join ?p ?a ?b) ?c)"
            if all(vec![reads_only_either("?p", "?a", "?b"), rejects_padding_of("?q", "?b")])),
        // (A ⟕p B) × C = (A × C) ⟕p B when p reads nothing of C.
        rw!("cross-left"; "(join (left-join ?p ?a ?b) ?c)" => "(left-join ?p (join ?a ?c) ?b)"),
        rw!("cross-left-rev"; "(left-join ?p (join ?a ?c) ?b)" => "(join (left-join ?p ?a ?b) ?c)"
            if reads_none_of("?p", "?c")),
        // Subqueries, as whole filters: there `IN` and `EXISTS` both pass a
        // row exactly when some subquery row has `c = x` TRUE.
        rw!("in-to-exists"; "(filter (in-sub ?x (project ?r ?c)) ?s)" => "(filter (exists (filter (= ?c ?x) ?r)) ?s)"),
        rw!("exists-to-in"; "(filter (exists (filter (= ?c ?x) ?r)) ?s)" => "(filter (in-sub ?x (project ?r ?c)) ?s)"
            if all(vec![uncorrelated("?r"), reads_only("?c", "?r"), reads_none_of("?x", "?r")])),
        // A key on the subquery side means no row of `s` matches twice.
        rw!("in-to-join"; "(filter (in-sub ?x (project ?r ?c)) ?s)" => "(filter (= ?c ?x) (join ?s ?r))"
            if all(vec![key_of("?c", "?r"), uncorrelated("?r")])),
        // DISTINCT.
        rw!("distinct-group-by"; "(distinct ?p)" => { DistinctAsGroupBy(var("?p")) }),
        rw!("distinct-by-key"; "(distinct ?p)" => "?p" if rows_distinct("?p")),
    ];
    rules.extend(rw!("and-assoc"; "(and ?a (and ?b ?c))" <=> "(and (and ?a ?b) ?c)"));
    rules.extend(rw!("or-assoc"; "(or ?a (or ?b ?c))" <=> "(or (or ?a ?b) ?c)"));
    rules.extend(rw!("lt-gt"; "(< ?a ?b)" <=> "(> ?b ?a)"));
    rules.extend(rw!("le-ge"; "(<= ?a ?b)" <=> "(>= ?b ?a)"));
    rules.extend(rw!("de-morgan-and"; "(not (and ?a ?b))" <=> "(or (not ?a) (not ?b))"));
    rules.extend(rw!("de-morgan-or"; "(not (or ?a ?b))" <=> "(and (not ?a) (not ?b))"));
    rules.extend(rw!("not-eq"; "(not (= ?a ?b))" <=> "(<> ?a ?b)"));
    rules.extend(rw!("not-lt"; "(not (< ?a ?b))" <=> "(>= ?a ?b)"));
    rules.extend(rw!("not-gt"; "(not (> ?a ?b))" <=> "(<= ?a ?b)"));
    rules.extend(rw!("distribute"; "(and ?a (or ?b ?c))" <=> "(or (and ?a ?b) (and ?a ?c))"));
    rules.extend(rw!("between"; "(between ?x ?a ?b)" <=> "(and (>= ?x ?a) (<= ?x ?b))"));
    rules.extend(rw!("in-2"; "(in ?x ?a ?b)" <=> "(or (= ?x ?a) (= ?x ?b))"));
    rules.extend(rw!("in-3"; "(in ?x ?a ?b ?c)" <=> "(or (= ?x ?a) (or (= ?x ?b) (= ?x ?c)))"));
    rules.extend(rw!("filter-merge"; "(filter ?p (filter ?q ?r))" <=> "(filter (and ?p ?q) ?r)"));
    rules.extend(rw!("join-assoc"; "(join (join ?a ?b) ?c)" <=> "(join ?a (join ?b ?c))"));
    rules
}
