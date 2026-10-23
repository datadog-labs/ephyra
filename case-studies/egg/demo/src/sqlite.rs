// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The demo's own check: a query and its variants on random SQLite databases.

use crate::{parse, sample::Rng};

/// For each variant, on how many random databases its rows (as a multiset)
/// differ from the seed's, and the error if it does not run at all.
pub fn check(
    input: &parse::Input,
    seed: &str,
    variants: &[&String],
    trials: usize,
    rng_seed: u64,
) -> Vec<(usize, Option<String>)> {
    let mut rng = Rng::new(rng_seed ^ 0x5eed);
    let mut outcomes: Vec<(usize, Option<String>)> = vec![(0, None); variants.len()];
    for _ in 0..trials {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        for ddl in &input.ddl {
            db.execute_batch(ddl).expect("the DDL runs in SQLite");
        }
        for table in &input.tables {
            for _ in 0..rng.next_u64() % 7 {
                let values: Vec<String> = table.columns.iter().map(|c| value(&mut rng, c, &input.literals)).collect();
                let sql = format!("INSERT OR IGNORE INTO {} VALUES ({})", table.name, values.join(", "));
                db.execute_batch(&sql).unwrap();
            }
        }
        let expected = rows(&db, seed).unwrap_or_else(|e| panic!("the seed does not run in SQLite: {e}"));
        for (k, v) in variants.iter().enumerate() {
            match rows(&db, v) {
                Ok(got) if got == expected => {},
                Ok(_) => outcomes[k].0 += 1,
                Err(e) => outcomes[k].1 = Some(e),
            }
        }
    }
    outcomes
}

/// A random value for a column: NULL sometimes (unless NOT NULL), else a small
/// number or one of the query's literals of the column's kind.
fn value(rng: &mut Rng, (_, ty, not_null): &(String, String, bool), literals: &[String]) -> String {
    if !not_null && rng.next_u64().is_multiple_of(5) {
        return "NULL".into();
    }
    // The query's own literals of the column's kind, and a few more. Small
    // integers on every INTEGER column, so foreign-key-style columns meet ids.
    let ty = ty.to_uppercase();
    let kind = |l: &str| if l.starts_with('\'') { "text" } else if l.contains('.') { "real" } else { "int" };
    let want = if ty.contains("INT") { "int" } else if ty.contains("REAL") { "real" } else { "text" };
    let mut pool: Vec<String> = literals.iter().filter(|l| kind(l) == want).cloned().collect();
    pool.extend(match want {
        "int" => (0..4).map(|i| i.to_string()).collect::<Vec<_>>(),
        "real" => ["0.5", "1.0", "2.5"].map(String::from).to_vec(),
        _ => ["'a'", "'b'"].map(String::from).to_vec(),
    });
    pool[(rng.next_u64() % pool.len() as u64) as usize].clone()
}

fn rows(db: &rusqlite::Connection, sql: &str) -> Result<Vec<String>, String> {
    let mut stmt = db.prepare(sql).map_err(|e| e.to_string())?;
    let n = stmt.column_count();
    let mut out: Vec<String> = stmt
        .query_map([], |row| Ok((0..n).map(|i| format!("{:?}", row.get_ref(i).unwrap())).collect::<Vec<_>>().join("|")))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    out.sort();
    Ok(out)
}
