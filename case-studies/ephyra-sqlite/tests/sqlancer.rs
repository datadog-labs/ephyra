//! The SQLancer-style metamorphic oracles, each a property over a case and
//! queries on the state it leaves: NoREC, TLP, differential query plans,
//! and in-memory against WAL-file storage. Their formulations and
//! exclusions are documented in `ephyra_sqlite::sqlancer`.

use ephyra::{Gen, Source};
use ephyra_sqlite::{
    cases::Case,
    expr::{Queries, Query, Schema},
    model::{Coll, Encoding},
    cases::Cases,
    probes::{case_queries, final_model, final_schema, repair_queries},
    sqlancer,
};

#[ephyra::test(gen = case_queries(), tests = 1500)]
fn norec_counts_agree(case: Case, queries: Vec<Query>) {
    if let Err(e) = sqlancer::norec(&case, &queries) {
        panic!("{e}");
    }
}

#[ephyra::test(gen = case_queries(), tests = 1500)]
fn tlp_partitions_agree(case: Case, queries: Vec<Query>) {
    if let Err(e) = sqlancer::tlp(&case, &queries) {
        panic!("{e}");
    }
}

#[ephyra::test(gen = case_queries(), tests = 1500)]
fn ordered_and_limited_rows_agree(case: Case, queries: Vec<Query>) {
    if let Err(e) = sqlancer::order(&case, &queries) {
        panic!("{e}");
    }
}

#[ephyra::test(gen = case_queries(), tests = 1500)]
fn set_operations_agree(case: Case, queries: Vec<Query>) {
    if let Err(e) = sqlancer::setops(&case, &queries) {
        panic!("{e}");
    }
}

#[ephyra::test(gen = case_queries(), tests = 1500)]
fn common_table_expressions_agree(case: Case, queries: Vec<Query>) {
    if let Err(e) = sqlancer::ctes(&case, &queries) {
        panic!("{e}");
    }
}

#[ephyra::test(gen = case_queries(), tests = 1500)]
fn window_functions_agree(case: Case, queries: Vec<Query>) {
    if let Err(e) = sqlancer::windows(&case, &queries) {
        panic!("{e}");
    }
}

#[ephyra::test(gen = case_queries(), tests = 1500)]
fn query_plans_agree(case: Case, queries: Vec<Query>) {
    if let Err(e) = sqlancer::plans(&case, &queries) {
        panic!("{e}");
    }
}

/// In memory and in a WAL file, every query answers alike (half the cases
/// reopen the file, so the WAL index is recovered); the files go with
/// `clear_storage`.
#[ephyra::test(gen = case_queries(), tests = 400)]
fn memory_and_wal_storage_agree(case: Case, queries: Vec<Query>) {
    let verdict = sqlancer::storage(&case, &queries);
    sqlancer::clear_storage();
    if let Err(e) = verdict {
        panic!("{e}");
    }
}

/// Every query drawn is one the oracles check in a UTF-16 database, where
/// [`sqlancer::concat_as_text`] rewrites a concat() argument that may be a
/// BLOB as `CAST(x AS TEXT)` ([`sqlancer::as_checked`]: no "a CTE of the
/// query is not valid"), here over every case's final tables with every
/// column NOCASE: run 20261016-1828 failed so on every oracle (fresh draws
/// too) once the rewrite was scoped instead of the query as drawn, a NOCASE
/// column cast to TEXT in an aggregate's argument in a CTE body failing
/// `casts_binary`. Rare as drawn (none in 590000 queries of inputs extended
/// to the runner's 1-300 queries, a few in a million), so a burn never
/// reaches it; with NOCASE columns a few per 10000 queries. No SQLite.
#[test]
fn every_query_is_checked_as_drawn_in_utf16_with_nocase_columns() {
    let gen = case_queries();
    let (mut queries, mut rewritten) = (0, 0);
    for seed in 0..3000u64 {
        let size = (seed % 101) as usize;
        let v = gen.generate(&mut Source::new(seed, size));
        let Some(mut m) = final_model(&v.0) else { continue };
        m.encoding = Encoding::Utf16le;
        for t in &mut m.tables {
            t.coll.iter_mut().for_each(|c| *c = Some(Coll::Nocase));
        }
        let schema = Schema::of(&m);
        let mut g = Source::new(seed ^ 0x5eed, size);
        for _ in 0..20 {
            for q in Queries(schema.clone()).generate(&mut g) {
                let (checked, _) = sqlancer::as_checked(&m, &schema, &q).unwrap_or_else(|e| panic!("seed {seed}: {e}"));
                queries += 1;
                rewritten += usize::from(matches!(checked, std::borrow::Cow::Owned(_)));
            }
        }
    }
    assert!(queries > 100_000 && rewritten > 3000, "{queries} queries, {rewritten} rewritten");
}

/// A seed's queries on a fresh case, repaired against its final schema (the
/// runner's `rare` mutants, `queries_on` in sqlite-coverage): an FTS5 query
/// drawn for a wider table names columns (`{s0 s2} : t12`) another table of
/// the same number may lack ("no such column: s2", run 20261021-1717 on
/// every oracle), or a table of `detail=none` takes no `s0 MATCH` ("fts5:
/// column queries are not supported (detail=none)"), so a query is valid
/// only on a table with every column its text names, and a column MATCH
/// only on one that takes column queries. Seeds 421 and 1190 failed so.
#[test]
fn queries_recased_name_only_columns_there() {
    let gen = case_queries();
    let (mut fts, mut kept) = (0, 0);
    for seed in 0..2000u64 {
        let size = (seed % 101) as usize;
        let v = gen.generate(&mut Source::new(seed, size));
        if !format!("{:?}", v.0).contains("fts5(") {
            continue;
        }
        fts += 1;
        let case = Cases.generate(&mut Source::new(seed ^ 0x002e_c45e, size));
        let schema = final_schema(&case);
        let qs: Vec<Query> = repair_queries(&schema, &v.1).into_iter().filter(|q| q.fits(&schema)).collect();
        if qs.is_empty() {
            continue;
        }
        kept += 1;
        if let Err(e) = sqlancer::norec(&case, &qs) {
            assert!(!e.starts_with("the original query fails"), "seed {seed}: {e}");
        }
    }
    assert!(fts > 800 && kept > 400, "{fts} with an FTS5 table, {kept} kept");
}
