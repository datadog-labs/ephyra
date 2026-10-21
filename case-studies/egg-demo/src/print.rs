//! A term back to SQL. Top-level filters become WHERE, except one right above
//! an inner join, which becomes its ON; a filter over one table becomes a
//! derived table that keeps the table's alias, so every `alias.column` still
//! resolves.
//!
//! The printer refuses rather than guess, and the sampler draws again: a
//! shape with no faithful rendering (a filter over a nested join that is not
//! its ON), and a column of an alias not in scope where it is printed. The
//! second happens because expressions equal everywhere share an e-class
//! whatever they read: `s.id IS NULL` on a key and `t.c IS NULL` on a NOT
//! NULL column are both FALSE, and either can stand for the other in the
//! e-graph, but only one of them names a table the query can see there.

use std::collections::BTreeSet;

use egg::{Id, RecExpr};

use crate::lang::{column_alias, Sql};

type Scope = BTreeSet<String>;

pub fn sql(term: &RecExpr<Sql>) -> Option<String> {
    query(term, Id::from(term.as_ref().len() - 1), &Scope::new())
}

/// A projection, `distinct` or `group-by` as a SELECT statement, under the
/// aliases `outer` of the enclosing queries.
fn query(term: &RecExpr<Sql>, id: Id, outer: &Scope) -> Option<String> {
    match &term[id] {
        Sql::Project(children) => select(term, children, "SELECT", false, outer),
        Sql::Distinct([p]) => match &term[*p] {
            Sql::Project(children) => select(term, children, "SELECT DISTINCT", false, outer),
            _ => None,
        },
        Sql::GroupBy(children) => select(term, children, "SELECT", true, outer),
        _ => None,
    }
}

fn with(outer: &Scope, more: Scope) -> Scope {
    outer.union(&more).cloned().collect()
}

fn select(term: &RecExpr<Sql>, children: &[Id], head: &str, group: bool, outer: &Scope) -> Option<String> {
    let scope = with(outer, provided(term, children[0]));
    let columns: Vec<String> = children[1..].iter().map(|&c| expr(term, c, &scope)).collect::<Option<_>>()?;
    let mut out = format!("{head} {} {}", columns.join(", "), body(term, children[0], outer)?);
    if group {
        out += &format!(" GROUP BY {}", columns.join(", "));
    }
    Some(out)
}

/// `FROM ... [WHERE ...]` for a relation.
fn body(term: &RecExpr<Sql>, mut rel: Id, outer: &Scope) -> Option<String> {
    let scope = with(outer, provided(term, rel));
    let mut conditions = Vec::new();
    while let Sql::Filter([p, inner]) = term[rel] {
        if matches!(term[inner], Sql::Join(_)) {
            break;
        }
        conditions.push(expr(term, p, &scope)?);
        rel = inner;
    }
    let mut out = format!("FROM {}", from(term, rel, outer)?);
    if !conditions.is_empty() {
        conditions.reverse();
        out += &format!(" WHERE {}", conditions.join(" AND "));
    }
    Some(out)
}

fn from(term: &RecExpr<Sql>, rel: Id, outer: &Scope) -> Option<String> {
    let joined = |a: &Id, b: &Id| with(outer, provided(term, *a).union(&provided(term, *b)).cloned().collect());
    match &term[rel] {
        Sql::Scan([table, alias]) => {
            let (t, a) = (symbol(term, *table)?, symbol(term, *alias)?);
            Some(if t == a { t } else { format!("{t} AS {a}") })
        },
        Sql::Join([a, b]) => Some(format!("{} CROSS JOIN {}", item(term, *a, outer)?, item(term, *b, outer)?)),
        Sql::LeftJoin([q, a, b]) => Some(format!(
            "{} LEFT JOIN {} ON {}",
            item(term, *a, outer)?,
            item(term, *b, outer)?,
            expr(term, *q, &joined(a, b))?
        )),
        Sql::Filter([p, inner]) => match &term[*inner] {
            Sql::Join([a, b]) => Some(format!(
                "{} JOIN {} ON {}",
                item(term, *a, outer)?,
                item(term, *b, outer)?,
                expr(term, *p, &joined(a, b))?
            )),
            _ => {
                let aliases = provided(term, *inner);
                let [alias] = aliases.iter().collect::<Vec<_>>()[..] else { return None };
                let scope = with(outer, aliases.clone());
                Some(format!(
                    "(SELECT * FROM {} WHERE {}) AS {alias}",
                    from(term, *inner, outer)?,
                    expr(term, *p, &scope)?
                ))
            },
        },
        _ => None,
    }
}

/// A FROM item inside a join: a join nested in it gets parentheses.
fn item(term: &RecExpr<Sql>, rel: Id, outer: &Scope) -> Option<String> {
    let is_join = match &term[rel] {
        Sql::Join(_) | Sql::LeftJoin(_) => true,
        Sql::Filter([_, inner]) => matches!(term[*inner], Sql::Join(_)),
        _ => false,
    };
    let s = from(term, rel, outer)?;
    Some(if is_join { format!("({s})") } else { s })
}

/// The aliases a relation provides.
fn provided(term: &RecExpr<Sql>, rel: Id) -> Scope {
    match &term[rel] {
        Sql::Scan([_, alias]) => symbol(term, *alias).into_iter().collect(),
        Sql::Join([a, b]) | Sql::LeftJoin([_, a, b]) => {
            provided(term, *a).union(&provided(term, *b)).cloned().collect()
        },
        Sql::Filter([_, inner]) => provided(term, *inner),
        _ => Scope::new(),
    }
}

fn symbol(term: &RecExpr<Sql>, id: Id) -> Option<String> {
    match &term[id] {
        Sql::Symbol(s) => Some(s.to_string()),
        _ => None,
    }
}

/// An expression, refusing a column of an alias not in `scope`.
fn expr(term: &RecExpr<Sql>, id: Id, scope: &Scope) -> Option<String> {
    let e = |id: &Id| expr(term, *id, scope);
    let binary = |op: &str, [a, b]: &[Id; 2]| Some(format!("({} {op} {})", e(a)?, e(b)?));
    match &term[id] {
        Sql::Symbol(s) => match column_alias(s.as_str()) {
            Some(alias) if !scope.contains(alias) => None,
            _ => Some(s.to_string()),
        },
        Sql::And(c) => binary("AND", c),
        Sql::Or(c) => binary("OR", c),
        Sql::Eq(c) => binary("=", c),
        Sql::Ne(c) => binary("<>", c),
        Sql::Lt(c) => binary("<", c),
        Sql::Le(c) => binary("<=", c),
        Sql::Gt(c) => binary(">", c),
        Sql::Ge(c) => binary(">=", c),
        Sql::Add(c) => binary("+", c),
        Sql::Not([a]) => Some(format!("(NOT {})", e(a)?)),
        Sql::IsNull([a]) => Some(format!("({} IS NULL)", e(a)?)),
        Sql::Between([x, a, b]) => Some(format!("({} BETWEEN {} AND {})", e(x)?, e(a)?, e(b)?)),
        Sql::In(items) => {
            let list: Vec<String> = items[1..].iter().map(e).collect::<Option<_>>()?;
            Some(format!("({} IN ({}))", e(&items[0])?, list.join(", ")))
        },
        Sql::InSub([x, sub]) => Some(format!("({} IN ({}))", e(x)?, query(term, *sub, scope)?)),
        Sql::Exists([rel]) => Some(format!("(EXISTS (SELECT * {}))", body(term, *rel, scope)?)),
        Sql::Project(_)
        | Sql::Distinct(_)
        | Sql::GroupBy(_)
        | Sql::Filter(_)
        | Sql::Join(_)
        | Sql::LeftJoin(_)
        | Sql::Scan(_) => None,
    }
}
