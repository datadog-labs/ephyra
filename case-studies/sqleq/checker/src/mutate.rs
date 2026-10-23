// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! One-operator mutants of a query, which most likely mean something else:
//! a comparison changed to its neighbour, AND for OR, a NULL test, IN or
//! EXISTS negated, DISTINCT dropped, a LEFT JOIN made inner. Whether a
//! mutant really differs is for PostgreSQL to say on the case's data.

use ephyra_postgresql::{
    expr::{children_mut, Expr, JoinKind, Query},
    statements::CmpOp,
};

use crate::{lang::slots_mut, sample::Rng};

fn sites(e: &Expr) -> usize {
    let mut n = 0;
    e.walk(&mut |x| n += usize::from(matches!(x, Expr::Cmp(..) | Expr::And(..) | Expr::Or(..) | Expr::IsNull(..) | Expr::InList(..) | Expr::Exists(..) | Expr::InQuery(..))));
    n
}

/// Mutate the `k`-th site of `e` (in walk order); returns the sites left.
fn flip(e: &mut Expr, k: &mut usize) -> bool {
    let here = matches!(e, Expr::Cmp(..) | Expr::And(..) | Expr::Or(..) | Expr::IsNull(..) | Expr::InList(..) | Expr::Exists(..) | Expr::InQuery(..));
    if here {
        if *k == 0 {
            let taken = std::mem::replace(e, Expr::null(ephyra_postgresql::model::Type::Bool));
            *e = match taken {
                Expr::Cmp(op, a, b) => Expr::Cmp(
                    match op {
                        CmpOp::Eq => CmpOp::Ne,
                        CmpOp::Ne => CmpOp::Eq,
                        CmpOp::Lt => CmpOp::Le,
                        CmpOp::Le => CmpOp::Lt,
                        CmpOp::Gt => CmpOp::Ge,
                        CmpOp::Ge => CmpOp::Gt,
                    },
                    a,
                    b,
                ),
                Expr::And(a, b) => Expr::Or(a, b),
                Expr::Or(a, b) => Expr::And(a, b),
                Expr::IsNull(a, n) => Expr::IsNull(a, !n),
                Expr::InList(a, l, n) => Expr::InList(a, l, !n),
                Expr::Exists(q, n) => Expr::Exists(q, !n),
                Expr::InQuery(a, q, n) => Expr::InQuery(a, q, !n),
                other => other,
            };
            return true;
        }
        *k -= 1;
    }
    children_mut(e).into_iter().any(|c| flip(c, k))
}

/// A mutant of `q`, or None when it has nothing to mutate.
pub fn mutant(q: &Query, rng: &mut Rng) -> Option<Query> {
    let mut m = q.clone();
    let expr_sites: usize = slots_mut(&mut m).iter().map(|e| sites(e)).sum();
    let lefts: Vec<usize> = (0..m.select.joins.len()).filter(|&j| m.select.joins[j].kind == JoinKind::Left).collect();
    let total = expr_sites + lefts.len() + usize::from(m.select.distinct);
    if total == 0 {
        return None;
    }
    let mut k = (rng.next_u64() % total as u64) as usize;
    if k < expr_sites {
        for e in slots_mut(&mut m) {
            if flip(e, &mut k) {
                break;
            }
        }
    } else if k - expr_sites < lefts.len() {
        m.select.joins[lefts[k - expr_sites]].kind = JoinKind::Inner;
    } else {
        m.select.distinct = false;
    }
    Some(m)
}
