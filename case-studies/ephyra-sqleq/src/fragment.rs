// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The seeds: ephyra-postgresql's queries, inside what sqleq's frontend
//! takes. sqleq refuses window functions, INTERSECT/EXCEPT ALL and
//! aggregates with an ORDER BY (`string_agg(DISTINCT x, ',' ORDER BY x)`) by
//! name, and its SQL parser does not read `BETWEEN SYMMETRIC` or `ceil`; a
//! query with any of them is no seed. ORDER BY and LIMIT are dropped (a
//! query's rows as a bag are what sqleq compares; LIMIT it refuses).

use ephyra_postgresql::expr::{AggFunc, Expr, Func, Query, SetOp};

use crate::lang::slots_mut;

fn expr_ok(e: &Expr) -> bool {
    let mut ok = true;
    e.walk(&mut |x| {
        ok &= !matches!(
            x,
            Expr::Window(_) | Expr::Agg(AggFunc::StringAgg, ..) | Expr::Between { symmetric: true, .. } | Expr::Func(Func::Ceil, _)
        );
        for q in x.subqueries() {
            ok &= query_ok(q);
        }
    });
    ok
}

fn query_ok(q: &Query) -> bool {
    let mut q = q.clone();
    q.compound.iter().all(|(op, _)| !matches!(op, SetOp::IntersectAll | SetOp::ExceptAll)) && slots_mut(&mut q).into_iter().all(|e| expr_ok(e))
}

/// The seed a generated query gives: without ORDER BY and LIMIT, or none.
pub fn seed(q: &Query) -> Option<Query> {
    if q.limit.is_some() {
        return None;
    }
    let q = Query { order: Vec::new(), ..q.clone() };
    query_ok(&q).then_some(q)
}
