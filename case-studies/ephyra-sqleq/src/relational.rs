// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Relational rewrites: equalities between queries that change their
//! shape, not one expression, applied one at a time (one-step variants, as
//! [`crate::onestep`] makes for the expression rules). Each holds in
//! PostgreSQL's bag semantics under the condition its function states;
//! every variant is still type-checked (`Query::valid`) and run on
//! PostgreSQL by the pipeline.
//!
//! - `rel-on-to-where`: `.. JOIN s ON p .. WHERE w` to `.. CROSS JOIN s ..
//!   WHERE (p) AND (w)`, and `rel-where-to-on` the other way (a top-level
//!   WHERE conjunct that reads only the sources up to an inner or cross join
//!   moves into its ON). An inner join's ON and the WHERE filter the same
//!   rows, unless a later RIGHT or FULL join null-extends the sources before
//!   the WHERE sees them: then neither moves.
//! - `rel-join-commute`: a two-source inner or cross join's sources swapped,
//!   every column of that level renumbered (subqueries' outer references
//!   included). Only with explicit items, which keep the column order.
//! - `rel-left-to-inner`: a LEFT JOIN made inner when a top-level WHERE
//!   conjunct is null-rejecting for the joined source (a comparison, BETWEEN,
//!   IN-list or IS NOT NULL with one of its columns as an operand), and no
//!   later RIGHT or FULL join.
//! - `rel-distinct-to-group`: `SELECT DISTINCT i..` to `SELECT i.. GROUP BY
//!   i..` (not aggregate, no window, no subquery among the items: both keep
//!   one row per class of equal rows), and `rel-group-to-distinct` back
//!   (GROUP BY exactly the items, no HAVING, no aggregate).
//! - `rel-in-to-exists`: a top-level WHERE conjunct `e IN (SELECT x FROM ..
//!   WHERE w)` to `EXISTS (SELECT x FROM .. WHERE (w) AND (x = e))`, `e`'s
//!   columns one level further out. In a WHERE conjunct only TRUE keeps a
//!   row, and IN is TRUE exactly when some row has `x = e` TRUE. Not for an
//!   aggregate or DISTINCT subquery (the filter would move below them), not
//!   for NOT IN (NULLs make it differ from NOT EXISTS).

use ephyra_postgresql::{
    expr::{children_mut, Col, Expr, JoinKind, Query, Schema, Select},
    statements::CmpOp,
};

use crate::lang::slots_mut;

/// (rule name, variant), each valid on `schema` and different from `q` as SQL.
pub fn relational(q: &Query, schema: &Schema) -> Vec<(String, Query)> {
    let mut out: Vec<(String, Query)> = Vec::new();
    let sql = q.sql();
    let mut push = |name: &str, v: Query| {
        if v.sql() != sql && v.valid(schema) && !out.iter().any(|(_, o)| o.sql() == v.sql()) {
            out.push((name.to_string(), v));
        }
    };
    for k in 0..selects(q) {
        let sel = select_at(q, k);
        for v in on_to_where(sel) {
            push("rel-on-to-where", with_select(q, k, v));
        }
        for v in where_to_on(sel) {
            push("rel-where-to-on", with_select(q, k, v));
        }
        if let Some(v) = join_commute(sel) {
            push("rel-join-commute", with_select(q, k, v));
        }
        for v in left_to_inner(sel) {
            push("rel-left-to-inner", with_select(q, k, v));
        }
        if let Some(v) = distinct_to_group(sel) {
            push("rel-distinct-to-group", with_select(q, k, v));
        }
        if let Some(v) = group_to_distinct(sel) {
            push("rel-group-to-distinct", with_select(q, k, v));
        }
        for v in in_to_exists(sel) {
            push("rel-in-to-exists", with_select(q, k, v));
        }
    }
    out
}

/// The query's own SELECTs (the top one, then compound members); CTE bodies
/// are left alone (renumbering their columns would change the CTE's).
fn selects(q: &Query) -> usize {
    1 + q.compound.len()
}

fn select_at(q: &Query, k: usize) -> &Select {
    if k == 0 {
        &q.select
    } else {
        &q.compound[k - 1].1
    }
}

fn with_select(q: &Query, k: usize, s: Select) -> Query {
    let mut v = q.clone();
    if k == 0 {
        v.select = s;
    } else {
        v.compound[k - 1].1 = s;
    }
    v
}

/// The top-level conjuncts of a predicate.
pub(crate) fn conjuncts(e: &Expr) -> Vec<Expr> {
    match e {
        Expr::And(a, b) => {
            let mut v = conjuncts(a);
            v.extend(conjuncts(b));
            v
        },
        _ => vec![e.clone()],
    }
}

pub(crate) fn and_all(mut cs: Vec<Expr>) -> Option<Expr> {
    let first = if cs.is_empty() { return None } else { cs.remove(0) };
    Some(cs.into_iter().fold(first, |a, b| Expr::And(Box::new(a), Box::new(b))))
}

/// Whether a later join than `k` (0-based among `joins`) is RIGHT or FULL.
fn later_outer(s: &Select, k: usize) -> bool {
    s.joins[k + 1..].iter().any(|j| matches!(j.kind, JoinKind::Right | JoinKind::Full))
}

fn on_to_where(s: &Select) -> Vec<Select> {
    let mut out = Vec::new();
    for k in 0..s.joins.len() {
        if s.joins[k].kind != JoinKind::Inner || later_outer(s, k) {
            continue;
        }
        let Some(on) = s.joins[k].on.clone() else { continue };
        let mut v = s.clone();
        v.joins[k].kind = JoinKind::Cross;
        v.joins[k].on = None;
        v.filter = Some(match v.filter.take() {
            Some(w) => Expr::And(Box::new(on), Box::new(w)),
            None => on,
        });
        out.push(v);
    }
    out
}

/// The highest source index of this level `e` reads (None: none), counting
/// columns inside subqueries that refer out to this level.
fn max_src(e: &Expr) -> Option<usize> {
    let mut m: Option<usize> = None;
    visit_cols(e, 0, &mut |c, depth| {
        if c.up == depth {
            m = Some(m.map_or(c.src, |x: usize| x.max(c.src)));
        }
    });
    m
}

fn where_to_on(s: &Select) -> Vec<Select> {
    let mut out = Vec::new();
    let Some(w) = &s.filter else { return out };
    let cs = conjuncts(w);
    for (ci, c) in cs.iter().enumerate() {
        // A conjunct with an aggregate or a subquery stays in the WHERE.
        if c.has_agg() || c.has_subquery() {
            continue;
        }
        let need = max_src(c).unwrap_or(0);
        for k in 0..s.joins.len() {
            // Join k brings in source k + 1; the conjunct may read sources up
            // to it.
            if need > k + 1 || !matches!(s.joins[k].kind, JoinKind::Inner | JoinKind::Cross) || later_outer(s, k) {
                continue;
            }
            let mut v = s.clone();
            let rest: Vec<Expr> = cs.iter().enumerate().filter(|(i, _)| *i != ci).map(|(_, e)| e.clone()).collect();
            v.filter = and_all(rest);
            v.joins[k].kind = JoinKind::Inner;
            v.joins[k].on = Some(match v.joins[k].on.take() {
                Some(on) => Expr::And(Box::new(on), Box::new(c.clone())),
                None => c.clone(),
            });
            out.push(v);
            break;
        }
    }
    out
}

/// Visit every column in `e`, with the number of subquery levels it is
/// below `e` (`depth`).
fn visit_cols(e: &Expr, depth: usize, f: &mut dyn FnMut(&Col, usize)) {
    if let Expr::Col(c) = e {
        f(c, depth);
    }
    for c in e.children() {
        visit_cols(c, depth, f);
    }
    if let Expr::Exists(q, _) | Expr::InQuery(_, q, _) | Expr::Scalar(q) = e {
        let mut q = (**q).clone();
        for slot in slots_mut(&mut q) {
            visit_cols(slot, depth + 1, f);
        }
    }
}

/// Rewrite every column in `e` with `f(col, depth)`, entering subqueries.
pub(crate) fn map_cols(e: &mut Expr, depth: usize, f: &dyn Fn(&mut Col, usize)) {
    if let Expr::Col(c) = e {
        f(c, depth);
    }
    if let Expr::Exists(q, _) | Expr::InQuery(_, q, _) | Expr::Scalar(q) = e {
        for slot in slots_mut(q) {
            map_cols(slot, depth + 1, f);
        }
    }
    for c in children_mut(e) {
        map_cols(c, depth, f);
    }
}

fn select_exprs_mut(s: &mut Select) -> Vec<&mut Expr> {
    let Select { items, joins, filter, group, having, .. } = s;
    let mut out: Vec<&mut Expr> = items.iter_mut().collect();
    out.extend(joins.iter_mut().filter_map(|j| j.on.as_mut()));
    out.extend(filter.iter_mut());
    out.extend(group.iter_mut());
    out.extend(having.iter_mut());
    out
}

fn join_commute(s: &Select) -> Option<Select> {
    if s.joins.len() != 1 || s.items.is_empty() || !matches!(s.joins[0].kind, JoinKind::Inner | JoinKind::Cross) {
        return None;
    }
    let mut v = s.clone();
    let (a, b) = (v.from.clone(), v.joins[0].src.clone());
    v.from = b;
    v.joins[0].src = a;
    for e in select_exprs_mut(&mut v) {
        map_cols(e, 0, &|c, depth| {
            if c.up == depth && c.src <= 1 {
                c.src = 1 - c.src;
            }
        });
    }
    Some(v)
}

/// Whether a conjunct is not TRUE whenever every column of source `src` (of
/// this level) is NULL: one of those columns is a direct operand of a
/// strict comparison.
fn null_rejecting(c: &Expr, src: usize) -> bool {
    let is_src = |e: &Expr| matches!(e, Expr::Col(x) if x.up == 0 && x.src == src);
    match c {
        Expr::Cmp(_, a, b) => is_src(a) || is_src(b),
        Expr::IsNull(a, true) => is_src(a),
        Expr::Between { e, .. } => is_src(e),
        Expr::InList(a, _, _) => is_src(a),
        Expr::Like { e, .. } => is_src(e),
        _ => false,
    }
}

fn left_to_inner(s: &Select) -> Vec<Select> {
    let mut out = Vec::new();
    let Some(w) = &s.filter else { return out };
    let cs = conjuncts(w);
    for k in 0..s.joins.len() {
        if s.joins[k].kind != JoinKind::Left || later_outer(s, k) {
            continue;
        }
        if cs.iter().any(|c| null_rejecting(c, k + 1)) {
            let mut v = s.clone();
            v.joins[k].kind = JoinKind::Inner;
            out.push(v);
        }
    }
    out
}

fn plain_items(s: &Select) -> bool {
    !s.items.is_empty() && s.items.iter().all(|e| !e.has_agg() && !e.has_subquery() && !matches!(e, Expr::Window(_)) && !ephyra_postgresql::expr::has_window(e))
}

fn distinct_to_group(s: &Select) -> Option<Select> {
    if !s.distinct || s.aggregate() || !plain_items(s) {
        return None;
    }
    let mut v = s.clone();
    v.distinct = false;
    v.group = s.items.clone();
    Some(v)
}

fn group_to_distinct(s: &Select) -> Option<Select> {
    if s.group.is_empty() || s.having.is_some() || !plain_items(s) {
        return None;
    }
    // GROUP BY exactly the items (as sets of expressions).
    let same = s.group.iter().all(|k| s.items.contains(k)) && s.items.iter().all(|i| s.group.contains(i));
    if !same {
        return None;
    }
    let mut v = s.clone();
    v.group.clear();
    v.distinct = true;
    Some(v)
}

/// `e` moved one subquery level in: every column of it refers one level
/// further out.
pub(crate) fn shifted_in(e: &Expr) -> Expr {
    let mut e = e.clone();
    map_cols(&mut e, 0, &|c, depth| {
        if c.up >= depth {
            c.up += 1;
        }
    });
    e
}

fn in_to_exists(s: &Select) -> Vec<Select> {
    let mut out = Vec::new();
    let Some(w) = &s.filter else { return out };
    let cs = conjuncts(w);
    for (ci, c) in cs.iter().enumerate() {
        let Expr::InQuery(needle, sub, false) = c else { continue };
        let inner = &sub.select;
        if inner.items.len() != 1 || inner.aggregate() || inner.distinct || !sub.compound.is_empty() || !sub.with.is_empty() {
            continue;
        }
        let x = inner.items[0].clone();
        let eq = Expr::Cmp(CmpOp::Eq, Box::new(x), Box::new(shifted_in(needle)));
        let mut q2 = (**sub).clone();
        q2.select.filter = Some(match q2.select.filter.take() {
            Some(f) => Expr::And(Box::new(f), Box::new(eq)),
            None => eq,
        });
        let mut v = s.clone();
        let mut cs2 = cs.clone();
        cs2[ci] = Expr::Exists(Box::new(q2), false);
        v.filter = and_all(cs2);
        out.push(v);
    }
    out
}
