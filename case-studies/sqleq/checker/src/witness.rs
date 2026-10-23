// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Witnesses: data on which PostgreSQL gives two queries different results.
//! A near-miss pair counts as non-equivalent only with a witness.
//!
//! The search starts from the case's own data (loaded as the oracles load
//! it), then tries small perturbations of each table the pair reads, each in
//! a transaction rolled back afterwards: set one nullable column NULL in
//! every row, insert a copy of a row with one nullable column NULL, insert a
//! row of defaults, duplicate a row, empty the table. These reach what the
//! anti-rules hinge on: NULLs, duplicates, unmatched and missing rows.
//! Then value-targeted ones, for boundaries (`<` against `<=`, a constant
//! off by one): each literal of the pair (and an integer one ±1) written to
//! every column of its type, in every row, into a copy of a row, and as a
//! new row of its own (for an empty table).
//! Results are compared as bags of values PostgreSQL compares equal
//! (`sqlancer::canon`); a perturbation that fails, or a run where either
//! query fails, is skipped.

use ephyra_postgresql::{
    cases::Case,
    expr::{Expr, Query},
    model::{Model, Type, Value},
    statements::literal,
    pg::{with_session, Cell, Session},
    sqlancer::{canon, prepare, run, Canon},
};

fn bag(rows: &[Vec<Cell>]) -> Vec<Vec<Canon>> {
    let mut b: Vec<Vec<Canon>> = rows.iter().map(|r| r.iter().map(canon).collect()).collect();
    b.sort();
    b
}

/// The perturbations to try, as SQL, for the tables `a` and `b` read.
pub fn perturbations(m: &Model, a: &Query, b: &Query) -> Vec<String> {
    let mut ids = crate::minimize::tables(a);
    ids.extend(crate::minimize::tables(b));
    ids.sort();
    ids.dedup();
    let lits = literals(a, b);
    let mut out = Vec::new();
    for t in m.tables.iter().filter(|t| ids.contains(&t.id)) {
        let name = format!("t{}", t.id);
        let nullable: Vec<u32> = t.columns.iter().filter(|c| !c.not_null && !c.identity && !t.key.contains(&c.id)).map(|c| c.id).collect();
        for &c in &nullable {
            out.push(format!("UPDATE {name} SET c{c} = NULL"));
            let cols: Vec<String> = t.columns.iter().map(|x| if x.id == c { "NULL".to_string() } else { format!("c{}", x.id) }).collect();
            out.push(format!("INSERT INTO {name} SELECT {} FROM {name} LIMIT 1", cols.join(", ")));
        }
        for (v, ty) in &lits {
            for c in t.columns.iter().filter(|c| comparable(c.ty, *ty)) {
                let val = format!("{}::{}", literal(v), c.ty.name());
                out.push(format!("UPDATE {name} SET c{} = {val}", c.id));
                let cols: Vec<String> = t.columns.iter().map(|x| if x.id == c.id { val.clone() } else { format!("c{}", x.id) }).collect();
                out.push(format!("INSERT INTO {name} SELECT {} FROM {name} LIMIT 1", cols.join(", ")));
                out.push(format!("INSERT INTO {name} (c{}) VALUES ({val})", c.id));
            }
        }
        out.push(format!("INSERT INTO {name} DEFAULT VALUES"));
        out.push(format!("INSERT INTO {name} SELECT * FROM {name} LIMIT 1"));
        out.push(format!("DELETE FROM {name}"));
    }
    out
}

/// Same numeric family, or the same type: a literal of `ty` written to a
/// column of `col` keeps its value.
fn comparable(col: Type, ty: Type) -> bool {
    col == ty || col.is_int() && ty.is_int()
}

/// Up to 8 distinct non-NULL literals of the pair (subqueries included),
/// integer ones also ±1.
fn literals(a: &Query, b: &Query) -> Vec<(Value, Type)> {
    let mut out: Vec<(Value, Type)> = Vec::new();
    let mut add = |v: Value, t: Type| {
        if v != Value::Null && t.admits(&v) && !out.iter().any(|(w, u)| *w == v && *u == t) {
            out.push((v, t));
        }
    };
    for q in [a, b] {
        let mut q = q.clone();
        for slot in crate::lang::slots_mut(&mut q) {
            walk_deep(slot, &mut |e| {
                if let Expr::Lit(v, t) = e {
                    add(v.clone(), *t);
                    if let Value::Int(n) = v {
                        for m in [n.checked_sub(1), n.checked_add(1)].into_iter().flatten() {
                            add(Value::Int(m), *t);
                        }
                    }
                }
            });
        }
    }
    out.truncate(8);
    out
}

/// Visit `e` and everything below it, subqueries included.
fn walk_deep(e: &Expr, f: &mut dyn FnMut(&Expr)) {
    e.walk(&mut |x| {
        f(x);
        for q in x.subqueries() {
            let mut q = q.clone();
            for slot in crate::lang::slots_mut(&mut q) {
                walk_deep(slot, f);
            }
        }
    });
}

/// The results of `a` and `b` differ (both run).
fn differ(s: &mut Session, a: &str, b: &str) -> bool {
    match (run(s, a), run(s, b)) {
        (Ok(x), Ok(y)) => bag(&x) != bag(&y),
        _ => false,
    }
}

/// A witness that `a` and `b` differ: `Some("")` for the case's own data,
/// `Some(sql)` for a perturbation of it, None when none is found.
pub fn witness(case: &Case, m: &Model, a: &Query, b: &Query) -> Result<Option<String>, String> {
    let (sa, sb) = (a.sql(), b.sql());
    with_session(|s| {
        prepare(s, case)?;
        if differ(s, &sa, &sb) {
            return Ok(Some(String::new()));
        }
        for p in perturbations(m, a, b) {
            if s.execute("BEGIN").is_err() {
                continue;
            }
            let found = s.execute(&p).is_ok() && differ(s, &sa, &sb);
            let _ = s.execute("ROLLBACK");
            if found {
                return Ok(Some(p));
            }
        }
        Ok(None)
    })
}

/// [`witness`] for several variants of one seed, the case loaded once.
pub fn witnesses(case: &Case, m: &Model, seed: &Query, variants: &[Query]) -> Result<Vec<Option<String>>, String> {
    let sa = seed.sql();
    with_session(|s| {
        prepare(s, case)?;
        let mut out = Vec::with_capacity(variants.len());
        for v in variants {
            let sb = v.sql();
            if differ(s, &sa, &sb) {
                out.push(Some(String::new()));
                continue;
            }
            let mut found = None;
            for p in perturbations(m, seed, v) {
                if s.execute("BEGIN").is_err() {
                    continue;
                }
                let hit = s.execute(&p).is_ok() && differ(s, &sa, &sb);
                let _ = s.execute("ROLLBACK");
                if hit {
                    found = Some(p);
                    break;
                }
            }
            out.push(found);
        }
        Ok(out)
    })
}

/// A deeper search, for pairs a prover proved and no single perturbation
/// refuted: every pair of perturbations, applied together, up to `limit`
/// combinations. A witness here is a proof of a non-equivalent pair.
pub fn deep_witness(case: &Case, m: &Model, a: &Query, b: &Query, limit: usize) -> Result<Option<String>, String> {
    let (sa, sb) = (a.sql(), b.sql());
    let ps = perturbations(m, a, b);
    with_session(|s| {
        prepare(s, case)?;
        let mut tried = 0;
        for i in 0..ps.len() {
            for j in 0..ps.len() {
                if i == j {
                    continue;
                }
                tried += 1;
                if tried > limit {
                    return Ok(None);
                }
                if s.execute("BEGIN").is_err() {
                    continue;
                }
                let hit = s.execute(&ps[i]).is_ok() && s.execute(&ps[j]).is_ok() && differ(s, &sa, &sb);
                let _ = s.execute("ROLLBACK");
                if hit {
                    return Ok(Some(format!("{}; {}", ps[i], ps[j])));
                }
            }
        }
        Ok(None)
    })
}

/// A small, self-contained witness: INSERTs into the schema of `m` (only
/// the tables the pair reads) on which `a` and `b` differ, from the case's
/// data with perturbation `w` applied, rows dropped one at a time while the
/// difference stays (greedy, then until nothing more drops).
pub fn minimal_rows(case: &Case, m: &Model, a: &Query, b: &Query, w: &str) -> Result<Vec<String>, String> {
    let mut ids = crate::minimize::tables(a);
    ids.extend(crate::minimize::tables(b));
    ids.sort();
    ids.dedup();
    let (sa, sb) = (a.sql(), b.sql());
    with_session(|s| {
        prepare(s, case)?;
        if !w.is_empty() {
            for stmt in w.split("; ") {
                s.execute(stmt)?;
            }
        }
        // Every row of the tables read, as INSERTs.
        let mut rows: Vec<String> = Vec::new();
        for t in m.tables.iter().filter(|t| ids.contains(&t.id)) {
            let cols: Vec<String> = t.columns.iter().map(|c| format!("c{}", c.id)).collect();
            let data = s.values(&format!("SELECT {} FROM t{}", cols.join(", "), t.id))?;
            for r in data {
                let vals: Vec<String> = r.iter().zip(&t.columns).map(|(v, c)| format!("{}::{}", literal(v), c.ty.name())).collect();
                rows.push(format!("INSERT INTO t{} ({}) VALUES ({})", t.id, cols.join(", "), vals.join(", ")));
            }
        }
        let schema = crate::ddl::schema(&Model { tables: m.tables.iter().filter(|t| ids.contains(&t.id)).cloned().collect(), ..m.clone() });
        let differs_on = |s: &mut Session, rows: &[String]| -> bool {
            if s.fresh().is_err() || s.execute(&schema).is_err() {
                return false;
            }
            rows.iter().all(|r| s.execute(r).is_ok()) && differ(s, &sa, &sb)
        };
        if !differs_on(s, &rows) {
            return Err("the witness does not reproduce on the plain schema".into());
        }
        loop {
            let before = rows.len();
            let mut i = 0;
            while i < rows.len() {
                let mut fewer = rows.clone();
                fewer.remove(i);
                if differs_on(s, &fewer) {
                    rows = fewer;
                } else {
                    i += 1;
                }
            }
            if rows.len() == before {
                break;
            }
        }
        Ok(rows)
    })
}
