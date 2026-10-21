//! Anti-rules: plausible rewrites that are NOT equalities, the traps a
//! careless prover or optimizer falls into, applied one at a time so a pair
//! differs semantically in one place. A pair is only evidence once a witness
//! shows it differs ([`crate::witness`]): a rewrite can happen to be sound in
//! a given query (an anti-rule over a NOT NULL column, a DISTINCT over a
//! key), and then the pair is equivalent and expected to be provable.
//!
//! - `anti-not-in-exists`: a top-level WHERE conjunct `e NOT IN (SELECT x
//!   ..)` to `NOT EXISTS (SELECT .. WHERE x = e)` (differs when the subquery
//!   yields a NULL `x`, or `e` is NULL).
//! - `anti-union-all`: `UNION` and `UNION ALL` swapped (duplicates).
//! - `anti-distinct`: DISTINCT dropped or added on a SELECT (duplicates).
//! - `anti-count`: `count(*)` to `count(col)` of a nullable column, `count(x)`
//!   to `count(*)` (NULLs), DISTINCT toggled in `count`/`sum`/`avg`.
//! - `anti-left-inner`: a LEFT JOIN made inner (unmatched rows).
//! - `anti-left-on-where`: a LEFT JOIN's ON predicate moved to the WHERE
//!   (the ON becomes TRUE: unmatched rows and NULL-extended rows differ).
//! - `anti-strict`: `<` and `<=` (or `>` and `>=`) swapped (equal values).
//! - `anti-not-flip`: `NOT (a < b)` to `a > b` (drops equality).
//! - `anti-case-not`: a one-branch searched CASE's branches swapped on `NOT
//!   p` (NULL conditions go to ELSE on both sides).
//! - `anti-const`: an integer literal compared with a column moved by one.
//! - `anti-coalesce-swap`, `anti-nullif-swap`: arguments swapped.

use ephyra_postgresql::{
    expr::{children_mut, AggFunc, Expr, JoinKind, Query, Schema, Select, SetOp},
    model::{Type, Value},
    statements::CmpOp,
};

use crate::{
    lang::slots_mut,
    relational::{and_all, conjuncts, shifted_in},
};

/// (rule name, variant): every anti-rule application, each valid on
/// `schema` and different from `q` as SQL.
pub fn anti(q: &Query, schema: &Schema) -> Vec<(String, Query)> {
    let mut out: Vec<(String, Query)> = Vec::new();
    let sql = q.sql();
    let mut push = |name: &str, v: Query| {
        if v.sql() != sql && v.valid(schema) && !out.iter().any(|(_, o)| o.sql() == v.sql()) {
            out.push((name.to_string(), v));
        }
    };
    // Set operators.
    for k in 0..q.compound.len() {
        let mut v = q.clone();
        v.compound[k].0 = match v.compound[k].0 {
            SetOp::Union => SetOp::UnionAll,
            SetOp::UnionAll => SetOp::Union,
            _ => continue,
        };
        push("anti-union-all", v);
    }
    // Per SELECT.
    for k in 0..=q.compound.len() {
        let sel = if k == 0 { &q.select } else { &q.compound[k - 1].1 };
        let with = |s: Select| {
            let mut v = q.clone();
            if k == 0 {
                v.select = s;
            } else {
                v.compound[k - 1].1 = s;
            }
            v
        };
        let mut s = sel.clone();
        s.distinct = !s.distinct;
        push("anti-distinct", with(s));
        for j in 0..sel.joins.len() {
            if sel.joins[j].kind != JoinKind::Left {
                continue;
            }
            let mut s = sel.clone();
            s.joins[j].kind = JoinKind::Inner;
            push("anti-left-inner", with(s));
            if let Some(on) = sel.joins[j].on.clone() {
                let mut s = sel.clone();
                s.joins[j].on = Some(Expr::Lit(Value::Bool(true), Type::Bool));
                s.filter = Some(match s.filter.take() {
                    Some(w) => Expr::And(Box::new(on), Box::new(w)),
                    None => on,
                });
                push("anti-left-on-where", with(s));
            }
        }
        for v in not_in_exists(sel) {
            push("anti-not-in-exists", with(v));
        }
    }
    // Expression sites, anywhere in the query.
    let n = expr_sites(q);
    for i in 0..n {
        for (name, v) in at_site(q, i) {
            push(name, v);
        }
    }
    out
}

fn not_in_exists(s: &Select) -> Vec<Select> {
    let mut out = Vec::new();
    let Some(w) = &s.filter else { return out };
    let cs = conjuncts(w);
    for (ci, c) in cs.iter().enumerate() {
        let Expr::InQuery(needle, sub, true) = c else { continue };
        let inner = &sub.select;
        if inner.items.len() != 1 || inner.aggregate() || !sub.compound.is_empty() || !sub.with.is_empty() {
            continue;
        }
        let eq = Expr::Cmp(CmpOp::Eq, Box::new(inner.items[0].clone()), Box::new(shifted_in(needle)));
        let mut q2 = (**sub).clone();
        q2.select.filter = Some(match q2.select.filter.take() {
            Some(f) => Expr::And(Box::new(f), Box::new(eq)),
            None => eq,
        });
        let mut v = s.clone();
        let mut cs2 = cs.clone();
        cs2[ci] = Expr::Exists(Box::new(q2), true);
        v.filter = and_all(cs2);
        out.push(v);
    }
    out
}

/// The anti-rules that may apply at one expression node.
fn applies(e: &Expr) -> bool {
    matches!(
        e,
        Expr::Agg(..) | Expr::Cmp(..) | Expr::Not(_) | Expr::Case { .. } | Expr::Coalesce(_) | Expr::NullIf(..) | Expr::Between { .. }
    )
}

fn count_sites(e: &Expr) -> usize {
    let mut n = usize::from(applies(e));
    for c in e.children() {
        n += count_sites(c);
    }
    n
}

fn expr_sites(q: &Query) -> usize {
    let mut q = q.clone();
    slots_mut(&mut q).iter().map(|e| count_sites(e)).sum()
}

/// The variants of `q` with an anti-rule applied at expression site `i` (in
/// walk order over the query's slots).
fn at_site(q: &Query, i: usize) -> Vec<(&'static str, Query)> {
    let mut q0 = q.clone();
    let mut k = i;
    let mut found: Option<Expr> = None;
    for slot in slots_mut(&mut q0) {
        if find(slot, &mut k, &mut found) {
            break;
        }
    }
    let Some(e) = found else { return Vec::new() };
    alternatives(&e, q)
        .into_iter()
        .map(|(name, alt)| {
            let mut v = q.clone();
            let mut k = i;
            for slot in slots_mut(&mut v) {
                if replace(slot, &mut k, &alt) {
                    break;
                }
            }
            (name, v)
        })
        .collect()
}

fn find(e: &mut Expr, k: &mut usize, found: &mut Option<Expr>) -> bool {
    if applies(e) {
        if *k == 0 {
            *found = Some(e.clone());
            return true;
        }
        *k -= 1;
    }
    children_mut(e).into_iter().any(|c| find(c, k, found))
}

fn replace(e: &mut Expr, k: &mut usize, alt: &Expr) -> bool {
    if applies(e) {
        if *k == 0 {
            *e = alt.clone();
            return true;
        }
        *k -= 1;
    }
    children_mut(e).into_iter().any(|c| replace(c, k, alt))
}

/// A nullable column of the query's first source, for `count(col)`.
fn nullable_col(q: &Query) -> Option<Expr> {
    q.select.items.iter().chain(q.select.filter.iter()).find_map(|e| {
        let mut found = None;
        e.walk(&mut |x| {
            if let Expr::Col(c) = x {
                if c.up == 0 && found.is_none() {
                    found = Some(x.clone());
                }
            }
        });
        found
    })
}

fn alternatives(e: &Expr, q: &Query) -> Vec<(&'static str, Expr)> {
    let b = |x: &Expr| Box::new(x.clone());
    let mut out = Vec::new();
    match e {
        Expr::Agg(AggFunc::Count, false, None) => {
            if let Some(c) = nullable_col(q) {
                out.push(("anti-count", Expr::Agg(AggFunc::Count, false, Some(Box::new(c)))));
            }
        },
        Expr::Agg(AggFunc::Count, d, Some(a)) => {
            out.push(("anti-count", Expr::Agg(AggFunc::Count, false, None)));
            out.push(("anti-count", Expr::Agg(AggFunc::Count, !d, Some(a.clone()))));
        },
        Expr::Agg(f @ (AggFunc::Sum | AggFunc::Avg), d, Some(a)) => out.push(("anti-count", Expr::Agg(*f, !d, Some(a.clone())))),
        Expr::Cmp(op, l, r) => {
            let swapped = match op {
                CmpOp::Lt => Some(CmpOp::Le),
                CmpOp::Le => Some(CmpOp::Lt),
                CmpOp::Gt => Some(CmpOp::Ge),
                CmpOp::Ge => Some(CmpOp::Gt),
                _ => None,
            };
            if let Some(o) = swapped {
                out.push(("anti-strict", Expr::Cmp(o, l.clone(), r.clone())));
            }
            // An integer literal against a column, moved by one.
            for (lit, col_left) in [(&**r, true), (&**l, false)] {
                if let Expr::Lit(Value::Int(n), t) = lit {
                    if let Some(m) = n.checked_add(1).filter(|m| t.admits(&Value::Int(*m))) {
                        let moved = Expr::Lit(Value::Int(m), *t);
                        out.push(("anti-const", if col_left { Expr::Cmp(*op, l.clone(), Box::new(moved)) } else { Expr::Cmp(*op, Box::new(moved), r.clone()) }));
                    }
                }
            }
        },
        Expr::Not(inner) => {
            if let Expr::Cmp(op, l, r) = &**inner {
                let flipped = match op {
                    CmpOp::Lt => Some(CmpOp::Gt),
                    CmpOp::Le => Some(CmpOp::Ge),
                    CmpOp::Gt => Some(CmpOp::Lt),
                    CmpOp::Ge => Some(CmpOp::Le),
                    _ => None,
                };
                if let Some(o) = flipped {
                    out.push(("anti-not-flip", Expr::Cmp(o, l.clone(), r.clone())));
                }
            }
        },
        Expr::Case { operand: None, whens, otherwise: Some(other) } if whens.len() == 1 => {
            let (p, t) = &whens[0];
            out.push(("anti-case-not", Expr::Case { operand: None, whens: vec![(Expr::Not(b(p)), (**other).clone())], otherwise: Some(b(t)) }));
        },
        Expr::Coalesce(xs) if xs.len() == 2 => out.push(("anti-coalesce-swap", Expr::Coalesce(vec![xs[1].clone(), xs[0].clone()]))),
        Expr::NullIf(a, c) => out.push(("anti-nullif-swap", Expr::NullIf(c.clone(), a.clone()))),
        Expr::Between { e: x, lo, hi, not, symmetric } => {
            if let Expr::Lit(Value::Int(n), t) = &**hi {
                if let Some(m) = n.checked_sub(1).filter(|m| t.admits(&Value::Int(*m))) {
                    out.push(("anti-const", Expr::Between { e: x.clone(), lo: lo.clone(), hi: Box::new(Expr::Lit(Value::Int(m), *t)), not: *not, symmetric: *symmetric }));
                }
            }
        },
        _ => {},
    }
    out
}
