// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The case generator, with its validity predicate, shrinker, mutator and
//! metrics.

use std::ops::ControlFlow;

use ephyra::{Gen, Source};

use crate::ast::{BINARY, Case, Expr, Literal, PREFIX, Query, Type};

/// Deepest filter allowed. Turso's `translate_expr` overflows the stack past
/// depth 100; we stay far below.
pub const MAX_DEPTH: usize = 6;

pub(crate) const TYPES: [Type; 3] = [Type::Integer, Type::Real, Type::Text];

/// A case is well formed when every row fits the schema, every reference
/// names a column, every REAL is finite, the filter is within
/// [`MAX_DEPTH`] and uses known operators, and `LIMIT` only comes with
/// `ORDER BY`.
#[ephyra::generator]
pub fn well_formed(case: &Case) -> bool {
    let arity = case.schema.len();
    arity > 0
        && case.rows.iter().all(|r| row_fits(r, &case.schema))
        && query_fits(&case.query, arity)
}

/// A value of the column's type, or NULL; REALs finite.
pub(crate) fn fits(v: &Literal, ty: Type) -> bool {
    match (v, ty) {
        (Literal::Null, _) | (Literal::Integer(_), Type::Integer) => true,
        (Literal::Text(_), Type::Text) => true,
        (Literal::Real(x), Type::Real) => x.is_finite(),
        _ => false,
    }
}

pub(crate) fn row_fits(row: &[Literal], schema: &[Type]) -> bool {
    row.len() == schema.len() && row.iter().zip(schema).all(|(v, t)| fits(v, *t))
}

pub(crate) fn query_fits(q: &Query, arity: usize) -> bool {
    q.columns.as_ref().is_none_or(|c| !c.is_empty() && c.iter().all(|&i| i < arity))
        && q.filter.as_ref().is_none_or(|p| filter_fits(p, arity))
        && (q.limit.is_none() || q.order.is_some())
}

/// Within [`MAX_DEPTH`], and fitting the table.
pub(crate) fn filter_fits(p: &Expr, arity: usize) -> bool {
    p.depth() <= MAX_DEPTH && expr_fits(p, arity)
}

fn expr_fits(e: &Expr, arity: usize) -> bool {
    let known = match e {
        Expr::Column(i) => *i < arity,
        Expr::Literal(Literal::Real(x)) => x.is_finite(),
        Expr::Prefix(op, _) => PREFIX.contains(op),
        Expr::Binary(op, _, _) => BINARY.contains(op),
        _ => true,
    };
    known && e.children().iter().all(|c| expr_fits(c, arity))
}

#[ephyra::generator(well_formed, metrics = case_metrics)]
pub fn cases() -> impl Gen<Value = Case> {
    CaseGen
}

pub struct CaseGen;

pub(crate) fn literal(g: &mut Source, ty: Type) -> Literal {
    match (ty, g.int(0..=9u8)) {
        (Type::Integer, 0) => Literal::Integer(*g.pick(&[i64::MIN, i64::MAX, -1, 0, 1])),
        (Type::Integer, 1) => Literal::Integer(g.int(..)),
        (Type::Integer, _) => Literal::Integer(g.int(-10..=10i64)),
        (Type::Real, 0) => Literal::Real(*g.pick(&[1e300, -1e300, 1e-300, 9.223372036854776e18])),
        (Type::Real, _) => Literal::Real(g.int(-40..=40i64) as f64 / 4.0),
        // Characters LIKE treats specially, and a case pair it folds.
        (Type::Text, _) => {
            let n = g.int(0..=4usize);
            Literal::Text((0..n).map(|_| *g.pick(&['a', 'b', 'A', '%', '_', ' '])).collect())
        },
    }
}

pub(crate) fn value(g: &mut Source, ty: Type) -> Literal {
    if g.bool_with(0.15) { Literal::Null } else { literal(g, ty) }
}

pub(crate) fn expr(g: &mut Source, schema: &[Type], depth: usize) -> Expr {
    if depth <= 1 || g.bool_with(0.3) {
        if g.bool_with(0.6) {
            return Expr::Column(g.index(schema.len()));
        }
        let ty = *g.pick(&TYPES);
        return Expr::Literal(value(g, ty));
    }
    let sub = |g: &mut Source| Box::new(expr(g, schema, depth - 1));
    match g.int(0..=4u8) {
        0 => Expr::Prefix(g.pick(&PREFIX), sub(g)),
        1 => Expr::IsNull(sub(g)),
        _ => Expr::Binary(g.pick(&BINARY), sub(g), sub(g)),
    }
}

pub(crate) fn query(g: &mut Source, schema: &[Type]) -> Query {
    let columns = g.bool().then(|| {
        let n = g.int(1..=schema.len());
        (0..n).map(|_| g.index(schema.len())).collect()
    });
    let depth = g.int(1..=4usize);
    let filter = g.bool_with(0.8).then(|| expr(g, schema, depth));
    let order = g.bool().then(|| g.bool());
    let limit = (order.is_some() && g.bool()).then(|| g.int(0..=5u32));
    Query { columns, filter, order, limit }
}

impl Gen for CaseGen {
    type Value = Case;

    fn generate(&self, g: &mut Source) -> Case {
        let arity = g.int(1..=4usize);
        let schema: Vec<Type> = (0..arity).map(|_| *g.pick(&TYPES)).collect();
        // Sizes grow with the logarithm of the test count, so rows scale by three.
        let n = g.int(0..=(3 * g.size()).min(40));
        let rows = (0..n).map(|_| schema.iter().map(|&ty| value(g, ty)).collect()).collect();
        let query = query(g, &schema);
        Case { schema, rows, query }
    }

    /// Clauses first, then rows, then values. Every candidate removes a
    /// clause, a projected column, a row or an expression node, or moves a
    /// literal toward zero or empty; none adds anything. Columns of the
    /// schema are not dropped yet: that needs references rewritten.
    fn shrink(
        &self,
        case: &Case,
        visit: &mut dyn FnMut(Case) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        for query in shrink_query(&case.query) {
            visit(Case { query, ..case.clone() })?;
        }
        let n = case.rows.len();
        if n > 2 {
            visit(Case { rows: case.rows[..n / 2].to_vec(), ..case.clone() })?;
        }
        for i in 0..n {
            let mut rows = case.rows.clone();
            rows.remove(i);
            visit(Case { rows, ..case.clone() })?;
        }
        for (r, row) in case.rows.iter().enumerate() {
            for (c, v) in row.iter().enumerate() {
                for v in shrink_literal(v) {
                    let mut rows = case.rows.clone();
                    rows[r][c] = v;
                    visit(Case { rows, ..case.clone() })?;
                }
            }
        }
        ControlFlow::Continue(())
    }

    /// One local change: a fresh query or filter, a changed value, or an
    /// added row.
    fn mutate(&self, case: &Case, g: &mut Source) -> Case {
        let mut case = case.clone();
        match g.int(0..=3u8) {
            0 => case.query = query(g, &case.schema),
            1 => {
                let depth = g.int(1..=4usize);
                case.query.filter = Some(expr(g, &case.schema, depth));
            },
            2 if !case.rows.is_empty() => {
                let (r, c) = (g.index(case.rows.len()), g.index(case.schema.len()));
                case.rows[r][c] = value(g, case.schema[c]);
            },
            _ => {
                let row = case.schema.iter().map(|&ty| value(g, ty)).collect();
                case.rows.push(row);
            },
        }
        case
    }
}

/// Drop `LIMIT` (or `ORDER BY` without one), the filter or a projected
/// column, or shrink the filter.
pub(crate) fn shrink_query(q: &Query) -> Vec<Query> {
    let mut out = Vec::new();
    if q.limit.is_some() {
        out.push(Query { limit: None, ..q.clone() });
    } else if q.order.is_some() {
        out.push(Query { order: None, ..q.clone() });
    }
    if let Some(p) = &q.filter {
        out.push(Query { filter: None, ..q.clone() });
        out.extend(shrink_expr(p).into_iter().map(|p| Query { filter: Some(p), ..q.clone() }));
    }
    if let Some(columns) = q.columns.as_ref().filter(|c| c.len() > 1) {
        for i in 0..columns.len() {
            let mut columns = columns.clone();
            columns.remove(i);
            out.push(Query { columns: Some(columns), ..q.clone() });
        }
    }
    out
}

/// A child, or the same node with a smaller child; a column becomes NULL.
pub(crate) fn shrink_expr(e: &Expr) -> Vec<Expr> {
    let mut out: Vec<Expr> = e.children().into_iter().cloned().collect();
    let smaller = |e: &Expr| shrink_expr(e).into_iter().map(Box::new);
    match e {
        Expr::Column(_) => out.push(Expr::Literal(Literal::Null)),
        Expr::Literal(l) => out.extend(shrink_literal(l).into_iter().map(Expr::Literal)),
        Expr::Prefix(op, a) => out.extend(smaller(a).map(|a| Expr::Prefix(op, a))),
        Expr::IsNull(a) => out.extend(smaller(a).map(Expr::IsNull)),
        Expr::Binary(op, a, b) => {
            out.extend(smaller(a).map(|a| Expr::Binary(op, a, b.clone())));
            out.extend(smaller(b).map(|b| Expr::Binary(op, a.clone(), b)));
        },
    }
    out
}

/// Toward zero or the empty string, keeping the type. NULL is final.
pub(crate) fn shrink_literal(v: &Literal) -> Vec<Literal> {
    fn toward<T: PartialEq>(v: &T, mut candidates: Vec<T>) -> Vec<T> {
        candidates.dedup();
        candidates.retain(|c| c != v);
        candidates
    }
    match v {
        Literal::Null => vec![],
        Literal::Integer(n) => {
            toward(n, vec![0, n / 2]).into_iter().map(Literal::Integer).collect()
        },
        Literal::Real(x) => toward(x, vec![0.0]).into_iter().map(Literal::Real).collect(),
        Literal::Text(s) => {
            let candidates = vec![String::new(), s.chars().skip(1).collect()];
            toward(s, candidates).into_iter().map(Literal::Text).collect()
        },
    }
}

/// - rows, columns: raw counts (columns in the schema).
/// - filter depth, filter nodes: 0 without a filter.
/// - null share: NULL row values over all row values; 0 with no rows.
/// - filter columns: distinct columns the filter reads; 0 means it is
///   constant.
/// - ordered, limited: indicators.
///
/// Predicate selectivity needs an engine, so it is not a metric here.
pub fn case_metrics(case: &Case, _size: usize) -> Vec<(&'static str, f64)> {
    let values = case.rows.len() * case.schema.len();
    let nulls = case.rows.iter().flatten().filter(|v| **v == Literal::Null).count();
    let filter = case.query.filter.as_ref();
    let mut read = Vec::new();
    if let Some(p) = filter {
        p.columns(&mut read);
    }
    read.sort_unstable();
    read.dedup();
    let flag = |b: bool| if b { 1.0 } else { 0.0 };
    vec![
        ("rows", case.rows.len() as f64),
        ("columns", case.schema.len() as f64),
        ("filter depth", filter.map_or(0, Expr::depth) as f64),
        ("filter nodes", filter.map_or(0, Expr::nodes) as f64),
        ("null share", if values == 0 { 0.0 } else { nulls as f64 / values as f64 }),
        ("filter columns", read.len() as f64),
        ("ordered", flag(case.query.order.is_some())),
        ("limited", flag(case.query.limit.is_some())),
    ]
}
