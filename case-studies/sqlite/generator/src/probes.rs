// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Cases with queries over their final state: the input of the
//! SQLancer-style oracles (`tests/sqlancer.rs`). A [`Case`] (a file and
//! statements) and queries ([`crate::expr::Query`]) that refer only to the
//! tables and columns that exist after the statements: the model
//! [`final_model`] gives, through [`crate::semantics::apply`].
//!
//! The generator [`case_queries`] draws with `gens::dependent` (the case,
//! then queries on its final schema) and never runs SQLite; its metrics
//! do.

use std::ops::ControlFlow;

use ephyra::{gens, gens::Dependent, Gen, Source};

use crate::{
    cases::{self, model_of, well_formed, Case, Cases},
    expr::{ast_metrics, queries_cost, shrink_queries, Queries, Query, Schema},
    model::Model,
    semantics,
};

/// The model after the case's statements (`None` when the file's model
/// does not read back).
pub fn final_model(case: &Case) -> Option<Model> {
    let mut m = model_of(&case.file)?;
    for s in &case.statements {
        semantics::apply(&mut m, s);
    }
    Some(m)
}

/// The schema queries may use after the case's statements (none when the
/// case is not well formed).
pub fn final_schema(case: &Case) -> Schema {
    final_model(case).map_or(Schema { tables: Vec::new(), ctes: Vec::new(), catalog: Default::default() }, |m| Schema::of(&m))
}

/// The nightly runner's seed mixer (sqlite-coverage `mix`).
pub fn mix(seed: u64, index: u64) -> u64 {
    let mut z = seed.wrapping_add(index.wrapping_mul(0x9e37_79b9_7f4a_7c15));
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// `input` given k queries in all, k log-uniform in `lo..=hi` by its seed
/// `s`, more drawn on the case's final schema from a source of their own;
/// left as it is with as many already or without a table. The nightly
/// runner's `extend` (sqlite-coverage src/bin/runner/worker.rs), which its
/// `gen` profiles apply with 1-300: the gates draw 1 to 6 queries an input
/// otherwise, which missed what the runner found in run 20261016-1828.
pub fn extend(input: &mut (Case, Vec<Query>), s: u64, size: usize, (lo, hi): (usize, usize)) {
    let u = (mix(s, 0x0e_7e4d) % 1_000_000) as f64 / 1_000_000.0;
    let k = ((lo as f64).ln() + u * ((hi as f64).ln() - (lo as f64).ln())).exp().round() as usize;
    let schema = final_schema(&input.0);
    if schema.tables.is_empty() || input.1.len() >= k {
        return;
    }
    let mut g = Source::new(mix(s, 0x0e_7e4e), size);
    while input.1.len() < k {
        let more = Queries(schema.clone()).generate(&mut g);
        if more.is_empty() {
            break;
        }
        input.1.extend(more);
    }
    input.1.truncate(k);
}

/// The case is well formed and every query is valid against its final
/// schema.
#[ephyra::generator]
pub fn probe_well_formed(v: &(Case, Vec<Query>)) -> bool {
    well_formed(&v.0) && {
        let schema = final_schema(&v.0);
        v.1.iter().all(|q| q.valid(&schema))
    }
}

/// Cases and 1 to 6 queries on their final state (none when no table is
/// left).
#[ephyra::generator(probe_well_formed, metrics = probe_metrics)]
pub fn case_queries() -> impl Gen<Value = (Case, Vec<Query>)> {
    CaseQueries(gens::dependent(Cases, queries_for))
}

fn queries_for(case: &Case) -> Queries {
    Queries(final_schema(case))
}

/// Generation is `gens::dependent(Cases, ..)`; shrinking and mutation are
/// its own, to repair queries rather than redraw them.
pub struct CaseQueries(Dependent<Cases, fn(&Case) -> Queries>);

/// The queries kept against a smaller case's final schema: a query whose
/// table is gone is dropped, a column that is gone becomes NULL. Never
/// costlier ([`Query::repair`]).
pub fn repair_queries(schema: &Schema, qs: &[Query]) -> Vec<Query> {
    qs.iter().filter_map(|q| q.repair(schema)).collect()
}

/// The measure every shrink step strictly decreases, lexicographically:
/// the case's ([`cases::measure`]), then the queries' total cost.
pub fn probe_measure(v: &(Case, Vec<Query>)) -> ((u64, u64), u64) {
    (cases::measure(&v.0), queries_cost(&v.1))
}

impl Gen for CaseQueries {
    type Value = (Case, Vec<Query>);

    fn generate(&self, g: &mut Source) -> Self::Value {
        self.0.generate(g)
    }

    /// First the queries, the case kept ([`shrink_queries`]: lower query
    /// cost); then the case, by [`Cases`]' own steps (each lower in the
    /// case's measure, every candidate well formed), with the queries
    /// repaired against the smaller case's final schema, which never
    /// raises their cost. So every candidate is lower in
    /// [`probe_measure`], a lexicographic order on natural numbers, and
    /// every chain ends.
    fn shrink(&self, value: &Self::Value, visit: &mut dyn FnMut(Self::Value) -> ControlFlow<()>) -> ControlFlow<()> {
        let (case, qs) = value;
        shrink_queries(qs, &final_schema(case), &mut |q| visit((case.clone(), q)))?;
        let here = cases::measure(case);
        Cases.shrink(case, &mut |c| {
            if cases::measure(&c) >= here {
                return ControlFlow::Continue(());
            }
            let schema = final_schema(&c);
            let q = repair_queries(&schema, qs);
            visit((c, q))
        })
    }

    /// Half the time the queries change ([`Queries`]' mutation); else the
    /// case changes ([`Cases`]' mutation) and the queries are repaired
    /// against its final schema, those its grown tables put over the row
    /// budget dropped ([`Query::fits`]), or drawn anew when none is left.
    fn mutate(&self, value: &Self::Value, g: &mut Source) -> Self::Value {
        let (case, qs) = value;
        if g.bool() {
            return (case.clone(), queries_for(case).mutate(qs, g));
        }
        let c = Cases.mutate(case, g);
        let gen = queries_for(&c);
        let q: Vec<Query> = repair_queries(&gen.0, qs).into_iter().filter(|q| q.fits(&gen.0)).collect();
        let q = if q.is_empty() {
            crate::fell_back(&crate::QUERY_FALLBACKS);
            gen.generate(g)
        } else {
            q
        };
        (c, q)
    }
}

/// Numbers about a case and its queries:
///
/// - [`ast_metrics`] of the queries (syntax: depth, nodes, operators,
///   storage classes, function calls, ...).
/// - statements: the case's statements; final tables: tables after them;
///   final rows: rows in the tables the queries read after them (their
///   subqueries' included, each table once); indexed queries: queries with
///   a source whose table has an index, where the query-plan oracle has
///   INDEXED BY variants to compare.
/// - From SQLite, running the case and the queries: query errors: queries
///   that fail; selected share: the mean, over queries with a filter and a
///   FROM clause with rows, of the share of its (joined) rows the filter
///   selects (0 without any); low means filters miss, high means they
///   select everything.
pub fn probe_metrics(v: &(Case, Vec<Query>), _size: usize) -> Vec<(&'static str, f64)> {
    let (case, qs) = v;
    let mut out = ast_metrics(qs);
    let m = final_model(case);
    let tables = m.as_ref().map_or(0, |m| m.tables.len());
    let mut queried: Vec<usize> = qs.iter().flat_map(Query::tables).collect();
    queried.sort_unstable();
    queried.dedup();
    let rows: usize = queried.iter().filter_map(|&t| m.as_ref()?.tables.get(t)).map(|t| t.rows.len()).sum();
    let indexed = qs
        .iter()
        .filter(|q| q.sources().iter().any(|s| m.as_ref().and_then(|m| m.tables.get(*s)).is_some_and(|t| !t.indexes.is_empty())))
        .count();
    let (errors, share) = crate::sqlancer::selectivity(case, qs);
    out.extend([
        ("statements", case.statements.len() as f64),
        ("final tables", tables as f64),
        ("final rows", rows as f64),
        ("indexed queries", indexed as f64),
        ("query errors", errors as f64),
        ("selected share", share),
    ]);
    out
}

/// The type-affinity and collation features a case and its queries hold,
/// for a probe of how often the generator reaches them (the
/// `examples/gap_probe.rs` shares, and `sqlite-coverage`'s `gapcov`):
///
/// - `decl <affinity>`: a column of a table that is not STRICT declared
///   with a type of that affinity (`decl none` for `BLOB`), in the file's
///   model or a `CREATE TABLE` statement; `decl any` for any of them;
/// - `converted write`: an INSERT or UPDATE value its column's affinity
///   stores as another value (`'12'` as 12, 5 as `'5'`);
/// - `converted compare`: a statement's comparison of a column with a
///   literal its affinity converts (text that reads as a number under a
///   numeric affinity, a number under TEXT);
/// - `coll <sequence>`: a column declared with `COLLATE` of that sequence;
///   `coll index`: an index column with a `COLLATE` of its own; `coll
///   unique`: a UNIQUE index with a column under NOCASE or RTRIM; `coll
///   key`: a WITHOUT ROWID primary key with such a column; `collated
///   compare`: a statement's comparison of a column under NOCASE or RTRIM
///   with a text literal (all in the file's model or a statement's table or
///   index); `query collate`: a query with `e COLLATE NOCASE|RTRIM`.
pub fn gap_features(case: &Case, queries: &[crate::expr::Query]) -> std::collections::BTreeSet<&'static str> {
    use crate::{
        model::{affinity, column_coll, index_coll, numeric_text, Affinity, Coll, Index, Key, Numeric, Table, Value},
        statements::{Pred, Statement},
    };
    let indexed = |t: &Table, i: &Index, out: &mut std::collections::BTreeSet<&'static str>| {
        if !i.coll.is_empty() {
            out.insert("coll index");
        }
        if i.unique && i.columns.iter().any(|&(c, _)| index_coll(t, i, c) != Coll::Binary) {
            out.insert("coll unique");
        }
    };
    let mut out = std::collections::BTreeSet::new();
    if queries.iter().any(crate::sqlancer::collates) {
        out.insert("query collate");
    }
    let Some(mut m) = model_of(&case.file) else {
        return out;
    };
    let declared = |t: &Table, out: &mut std::collections::BTreeSet<&'static str>| {
        for k in t.coll.iter().flatten() {
            out.insert(match k {
                Coll::Binary => "coll binary",
                Coll::Nocase => "coll nocase",
                Coll::Rtrim => "coll rtrim",
            });
        }
        if let Key::WithoutRowid(pk) = &t.key {
            if pk.iter().any(|&c| column_coll(t, c) != Coll::Binary) {
                out.insert("coll key");
            }
        }
        for i in &t.indexes {
            indexed(t, i, out);
        }
        if t.strict.is_some() {
            return;
        }
        for c in 0..t.columns {
            if t.decl[c].is_none() || matches!(t.key, Key::IntegerPrimaryKey(k) if k == c) {
                continue;
            }
            out.insert("decl any");
            out.insert(match affinity(t, c) {
                Affinity::None => "decl none",
                Affinity::Text => "decl text",
                Affinity::Numeric => "decl numeric",
                Affinity::Integer => "decl integer",
                Affinity::Real => "decl real",
            });
        }
    };
    for t in &m.tables {
        declared(t, &mut out);
    }
    fn leaves<'a>(p: &'a Pred, out: &mut Vec<(usize, &'a Value)>) {
        match p {
            Pred::Cmp(c, _, v) => out.push((*c, v)),
            Pred::Between(c, lo, hi) => {
                out.push((*c, lo));
                out.push((*c, hi));
            },
            Pred::IsNull(..) => {},
            Pred::And(a, b) | Pred::Or(a, b) => {
                leaves(a, out);
                leaves(b, out);
            },
            Pred::Not(a) => leaves(a, out),
        }
    }
    for s in &case.statements {
        if !semantics::valid(&m, s) {
            break;
        }
        let (table, filter) = match s {
            Statement::Select(x) => (Some(x.table), x.filter.as_ref()),
            Statement::Update(x) => (Some(x.table), x.filter.as_ref()),
            Statement::Delete(x) => (Some(x.table), x.filter.as_ref()),
            Statement::Count { table, filter } => (Some(*table), filter.as_ref()),
            _ => (None, None),
        };
        if let (Some(t), Some(p)) = (table.and_then(|k| m.tables.get(k)), filter) {
            let mut ls = Vec::new();
            leaves(p, &mut ls);
            let crosses = |&(c, v): &(usize, &Value)| match (affinity(t, c), v) {
                (a, Value::Text(s)) if a.numeric() => matches!(numeric_text(s), Numeric::Int(_) | Numeric::Real(_)),
                (Affinity::Text, Value::Int(_) | Value::Real(_)) => true,
                _ => false,
            };
            if ls.iter().any(crosses) {
                out.insert("converted compare");
            }
            if ls.iter().any(|&(c, v)| matches!(v, Value::Text(_)) && column_coll(t, c) != Coll::Binary) {
                out.insert("collated compare");
            }
        }
        let converted = |t: &Table, c: usize, v: &Value| semantics::convert(t, c, v).is_some_and(|w| w != *v);
        match s {
            Statement::Insert(i) => {
                let t = &m.tables[i.table];
                let skip = usize::from(i.rowid);
                if i.rows.iter().any(|r| i.columns.iter().enumerate().any(|(k, &c)| !semantics::is_key(t, c) && converted(t, c, &r[skip + k]))) {
                    out.insert("converted write");
                }
            },
            Statement::Update(u) => {
                let t = &m.tables[u.table];
                if u.sets.iter().any(|(c, v)| converted(t, *c, v)) {
                    out.insert("converted write");
                }
            },
            Statement::CreateTable { def, .. } => declared(def, &mut out),
            Statement::CreateIndex { table, def, .. } => indexed(&m.tables[*table], def, &mut out),
            _ => {},
        }
        semantics::apply(&mut m, s);
    }
    out
}
