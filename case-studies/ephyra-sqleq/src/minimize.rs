// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Smaller pairs that sqleq still fails to prove. Every rule is a pointwise
//! equality of expressions, so a seed and a variant stay equivalent under
//! any change made to both alike, and a variant stays equivalent with any
//! of its slots put back as the seed has it. So:
//!
//! 1. slots of the variant are put back one at a time while the pair stays
//!    unproved;
//! 2. the seed is shrunk with ephyra-postgresql's query shrinker
//!    (`expr::simpler_queries`), each candidate's variants drawn anew, a
//!    candidate kept when one of its variants is unproved; then 1 again;
//! 3. the schema keeps only the tables the pair reads.
//!
//! "Unproved": the frontend lowers the pair and neither QED nor
//! `sqleq-solver` proves it.

use std::{
    path::Path,
    time::{Duration, Instant},
};

use ephyra_postgresql::{
    expr::{simpler_queries, Query, Schema, Src},
    model::Model,
};

use crate::{
    check::{check_with, Pair},
    ddl,
    lang::slots_mut,
    variants,
};

/// The tables a query reads (subqueries and CTEs included).
pub(crate) fn tables(q: &Query) -> Vec<u32> {
    let mut out = Vec::new();
    let mut sel = |s: &ephyra_postgresql::expr::Select| {
        for src in s.sources() {
            if let Src::Table(t) = src {
                out.push(*t);
            }
        }
    };
    for c in &q.with {
        sel(&c.query);
    }
    for s in q.selects() {
        sel(s);
    }
    let mut q2 = q.clone();
    let mut subs = Vec::new();
    for e in slots_mut(&mut q2) {
        e.walk(&mut |x| subs.extend(x.subqueries().into_iter().cloned()));
    }
    for s in subs {
        out.extend(tables(&s));
    }
    out.sort();
    out.dedup();
    out
}

/// The model with only the tables `a` and `b` read.
pub fn narrowed(m: &Model, a: &Query, b: &Query) -> Model {
    let mut keep = tables(a);
    keep.extend(tables(b));
    Model { tables: m.tables.iter().filter(|t| keep.contains(&t.id)).cloned().collect(), ..m.clone() }
}

pub fn unproved(m: &Model, a: &Query, b: &Query, dir: &Path) -> bool {
    let pair = Pair { name: "p".into(), file_text: ddl::pair_file(&narrowed(m, a, b), &a.sql(), &b.sql(), "minimize"), equivalent: true, pg_refutes: None };
    let row = check_with(vec![pair], dir, 1, false).remove(0);
    matches!(row.lowered, Some(Ok(()))) && !row.proved()
}

fn revert_slots(m: &Model, a: &Query, mut b: Query, dir: &Path) -> Query {
    let n = slots_mut(&mut b.clone()).len();
    for k in 0..n {
        let mut a2 = a.clone();
        let mut c = b.clone();
        let from = slots_mut(&mut a2)[k].clone();
        let mut to = slots_mut(&mut c);
        if *to[k] == from {
            continue;
        }
        *to[k] = from;
        if c.sql() != a.sql() && unproved(m, a, &c, dir) {
            b = c;
        }
    }
    b
}

/// Where QED panics on the pair (`src/pipeline/normal.rs:655`), if it does.
pub fn qed_panic_site(m: &Model, a: &Query, b: &Query, dir: &Path) -> Option<String> {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir.join("q")).ok()?;
    let file = dir.join("p.sql");
    std::fs::write(&file, ddl::pair_file(&narrowed(m, a, b), &a.sql(), &b.sql(), "panic")).ok()?;
    crate::tools::lower(&file, &dir.join("q").join("p.json")).ok()?;
    let out = std::process::Command::new(crate::tools::qed()).arg(dir.join("q")).output().ok()?;
    let err = String::from_utf8_lossy(&out.stderr);
    let at = err.lines().find(|l| l.contains("panicked at"))?;
    let site = at.split("panicked at ").nth(1)?.split(' ').next()?;
    // file:line, without the column.
    Some(site.rsplitn(2, ':').nth(1)?.trim_end_matches(':').to_string())
}

/// [`minimize_rule`] for any property of the pair (shrunk seeds keep a
/// one-step variant by `rule` for which `holds`).
pub fn minimize_rule_with(schema: &Schema, a: Query, b: Query, rule: &str, budget: Duration, holds: &dyn Fn(&Query, &Query) -> bool) -> (Query, Query) {
    let deadline = Instant::now() + budget;
    let (mut a, mut b) = (a, b);
    'outer: while Instant::now() < deadline {
        for c in simpler_queries(&a) {
            if Instant::now() > deadline {
                break 'outer;
            }
            if !c.valid(schema) || c.cost() >= a.cost() {
                continue;
            }
            if let Some((_, v)) = crate::onestep::one_step(&c, schema).into_iter().find(|(r, v)| r == rule && holds(&c, v)) {
                a = c;
                b = v;
                continue 'outer;
            }
        }
        break;
    }
    (a, b)
}

/// A smaller unproved pair (seed, variant), within `budget`; `rule` set:
/// a one-step pair, whose shrunk seeds keep a one-step variant by that
/// rule (and whose variant is never reverted slot by slot: it differs in
/// one place).
pub fn minimize_rule(m: &Model, schema: &Schema, a: Query, b: Query, rule: &str, dir: &Path, budget: Duration) -> (Query, Query) {
    let deadline = Instant::now() + budget;
    let (mut a, mut b) = (a, b);
    'outer: while Instant::now() < deadline {
        for c in simpler_queries(&a) {
            if Instant::now() > deadline {
                break 'outer;
            }
            if !c.valid(schema) || c.cost() >= a.cost() {
                continue;
            }
            let vs = crate::onestep::one_step(&c, schema);
            if let Some((_, v)) = vs.into_iter().find(|(r, v)| r == rule && unproved(m, &c, v, dir)) {
                a = c;
                b = v;
                continue 'outer;
            }
        }
        break;
    }
    (a, b)
}

/// A smaller unproved pair (seed, variant), within `budget`.
pub fn minimize(m: &Model, schema: &Schema, a: Query, b: Query, dir: &Path, budget: Duration) -> (Query, Query) {
    let deadline = Instant::now() + budget;
    let mut b = revert_slots(m, &a, b, dir);
    let mut a = a;
    'outer: while Instant::now() < deadline {
        for c in simpler_queries(&a) {
            if Instant::now() > deadline {
                break 'outer;
            }
            if !c.valid(schema) || c.cost() >= a.cost() {
                continue;
            }
            let (vs, _) = variants::variants(&c, schema, 8, 60, 1);
            if let Some(v) = vs.into_iter().find(|v| unproved(m, &c, v, dir)) {
                a = c;
                b = revert_slots(m, &a, v, dir);
                continue 'outer;
            }
        }
        break;
    }
    (a, b)
}
