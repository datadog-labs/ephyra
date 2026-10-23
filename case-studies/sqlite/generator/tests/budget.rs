// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The row budget ([`Query::fits`]): every query the generator draws reads
//! within it, and mutation keeps it so (a mutated case's grown tables drop
//! the queries they put over it).

use ephyra::{Gen, Source};
use ephyra_sqlite::{
    cases::Case,
    expr::Query,
    probes::{case_queries, final_schema},
};

#[ephyra::test(gen = case_queries(), tests = 3000)]
fn drawn_queries_fit_the_row_budget(case: Case, queries: Vec<Query>) {
    let schema = final_schema(&case);
    for q in &queries {
        assert!(q.fits(&schema), "{q:?}");
    }
}

#[test]
fn mutated_queries_fit_the_row_budget() {
    let gen = case_queries();
    for seed in 0..600 {
        let mut v = gen.generate(&mut Source::new(seed, (seed % 101) as usize));
        for round in 0..5u64 {
            v = gen.mutate(&v, &mut Source::new(seed * 7 + round, (seed % 101) as usize));
            let schema = final_schema(&v.0);
            for q in &v.1 {
                assert!(q.fits(&schema), "seed {seed}, round {round}: {q:?}");
            }
        }
    }
}

/// A virtual table's row count, as a source, is at least what SQLite
/// lists in it on the case's database, so that the row budget holds for
/// joins of them: drawn below it, joins of three such (`dbstat`,
/// `pragma_table_list`) made millions of rows, which the oracles collected
/// past 3 GB (runs 20261017-0438 and 20261018-0438: workers killed over
/// 8192 MB). Over inputs extended to 1-300 queries, as the runner's are.
#[test]
fn virtual_tables_count_what_sqlite_lists() {
    use ephyra_sqlite::{expr::CteBody, probes::extend, sqlancer};
    let gen = case_queries();
    let mut checked = 0;
    for seed in 9_300_000..9_300_400u64 {
        let size = (seed % 101) as usize;
        let mut v = gen.generate(&mut Source::new(seed, size));
        extend(&mut v, seed, size, (1, 300));
        let schema = final_schema(&v.0);
        let Ok(db) = sqlancer::open(&v.0) else { continue };
        for c in v.1.iter().flat_map(|q| &q.with) {
            let (CteBody::Vtab(vt), Some(info)) = (&c.body, c.info(&schema)) else { continue };
            let Ok(rows) = sqlancer::run(&db, &format!("SELECT count(*) FROM ({})", vt.sql(false))) else { continue };
            let [row] = rows.as_slice() else { continue };
            let sqlancer::Cell::Int(n) = row[0] else { continue };
            assert!(n as usize <= info.rows.max(1), "seed {seed}: {} gave {n} rows, counted {}", vt.sql(false), info.rows);
            checked += 1;
        }
    }
    assert!(checked > 1000, "{checked}");
}
