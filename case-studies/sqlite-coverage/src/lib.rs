//! The SQLite code that ephyra-sqlite's cases exercise, read in-process from
//! an instrumented SQLite (see `.cargo/config.toml`), and the control that
//! separates what a generated file's layout adds from what its contents and
//! statements reach on any file.

use std::{
    collections::{BTreeSet, HashMap},
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

use ephyra::{
    coverage::{self, Functions},
    Gen, Source,
};
use ephyra_sqlite::{
    cases::Case,
    expr::{Queries, Query},
    probes::{final_schema, repair_queries},
    reader, sqlancer,
    statements::Statement,
    SqliteFile, QUERY_FALLBACKS,
};
use rusqlite::Connection;

mod control;

pub use control::control;

/// The amalgamation's files that read, write and check the file format:
/// the B-tree, pager, WAL, page cache, record decoding and the VDBE that
/// runs the statements. The parser, planner and code generator are left out:
/// counting them would reward elaborate SQL rather than unusual files.
pub const FORMAT: &[&str] = &[
    "btree.c",
    "pager.c",
    "wal.c",
    "pcache.c",
    "pcache1.c",
    "memdb.c",
    "bitvec.c",
    "vdbe.c",
    "vdbeaux.c",
    "vdbemem.c",
    "utf.c",
    "util.c",
];

/// The counters of every function defined in a `FORMAT` file, by the
/// function-to-file map of the bundled (trunk) amalgamation (`sections.tsv`).
pub fn format_counters() -> BTreeSet<usize> {
    let sections: HashMap<&str, &str> = include_str!("../sections.tsv")
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.split_once('\t'))
        .collect();
    Functions::get()
        .all()
        .iter()
        .filter(|f| {
            let name = f.name.rsplit(':').next().unwrap_or(&f.name);
            sections.get(name).is_some_and(|s| FORMAT.contains(s))
        })
        .flat_map(|f| f.counters.clone())
        .collect()
}

/// Amalgamation files the oracles cannot reach by design: other platforms,
/// and extensions the cases never load or enable (as `deploy/gaps.py`).
/// FTS5, R-Tree and Geopoly are in scope since scorer v2 (2026-10-18): the
/// build enables them and cases can create their virtual tables.
pub const OUT_OF_SCOPE: &[&str] = &[
    "os_win.c", "os_kv.c", "mutex_w32.c", "mutex_noop.c", "mem0.c", "mem2.c", "mem3.c", "mem5.c", "fts3.c", "fts3_aux.c",
    "fts3_expr.c", "fts3_hash.c", "fts3_icu.c", "fts3_porter.c", "fts3_snippet.c", "fts3_tokenize_vtab.c", "fts3_tokenizer.c",
    "fts3_tokenizer1.c", "fts3_unicode.c", "fts3_unicode2.c", "fts3_write.c", "icu.c",
    "sqlite3session.c", "sqlite3rbu.c", "stmt.c", "carray.c", "test_vfstrace.c", "loadext.c", "notify.c", "userauth.c",
];

/// Every counter of a function defined in an amalgamation file in scope (not
/// [`OUT_OF_SCOPE`]), with that file, by the function-to-file map
/// (`sections.tsv`).
pub fn scoped_counters() -> HashMap<usize, &'static str> {
    let sections: HashMap<&str, &str> = include_str!("../sections.tsv")
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.split_once('\t'))
        .collect();
    let mut out = HashMap::new();
    for f in Functions::get().all() {
        let name = f.name.rsplit(':').next().unwrap_or(&f.name);
        if let Some(&file) = sections.get(name) {
            if !OUT_OF_SCOPE.contains(&file) {
                for c in f.counters.clone() {
                    out.insert(c, file);
                }
            }
        }
    }
    out
}

/// `file` deserialized into an in-memory database. A header saying WAL mode
/// is presented as rollback mode, as `integrity_ok` does.
pub fn open(file: &SqliteFile, read_only: bool) -> Connection {
    let mut db = Connection::open_in_memory().unwrap();
    if !file.0.is_empty() {
        let mut bytes = file.0.clone();
        for at in [18, 19] {
            if bytes[at] == 2 {
                bytes[at] = 1;
            }
        }
        db.deserialize_read_exact(rusqlite::MAIN_DB, &bytes[..], bytes.len(), read_only)
            .unwrap();
    }
    db
}

/// Run `sql` on `file` opened writable, reading every column of every row;
/// the number of statements that failed.
pub fn run(file: &SqliteFile, sql: &[String]) -> usize {
    let db = open(file, false);
    let mut failed = 0;
    for s in sql {
        let ok = db.prepare(s).and_then(|mut stmt| {
            let n = stmt.column_count();
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                for c in 0..n {
                    row.get_ref(c)?;
                }
            }
            Ok(())
        });
        failed += usize::from(ok.is_err());
    }
    failed
}

/// The `format` counters that running `sql` on `file` executes.
pub fn covered(format: &BTreeSet<usize>, file: &SqliteFile, sql: &[String]) -> BTreeSet<usize> {
    let before = coverage::snapshot().expect("an instrumented SQLite");
    run(file, sql);
    let delta = before.delta(&coverage::snapshot().expect("an instrumented SQLite"));
    delta
        .hits()
        .iter()
        .map(|(c, _)| *c)
        .filter(|c| format.contains(c))
        .collect()
}

/// What a case reaches beyond its control: the `format` counters its own
/// file executes under its statements that the control file, holding the
/// same rows, does not. Only the statements up to and including the first
/// VACUUM count: VACUUM reads the generated layout as it rewrites the file
/// in SQLite's own, so everything after it runs on a file the control could
/// have been.
pub fn beyond(format: &BTreeSet<usize>, case: &Case) -> BTreeSet<usize> {
    let end = case
        .statements
        .iter()
        .position(|s| matches!(s, Statement::Vacuum))
        .map_or(case.statements.len(), |i| i + 1);
    let sql: Vec<String> = case.statements[..end]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let Some((model, layout)) = reader::read(&case.file.0) else {
        return BTreeSet::new();
    };
    let mine = covered(format, &case.file, &sql);
    let theirs = covered(format, &control(&model, &layout), &sql);
    mine.difference(&theirs).copied().collect()
}

/// An oracle input (a case and queries on its final state) exercised
/// plainly, under a 5-second deadline: the case's file opened, its
/// statements run, each query run once. Its SQLite counters, sorted; `None`
/// when it ran past the deadline.
pub fn exercise(input: &(Case, Vec<Query>)) -> Option<Vec<u32>> {
    sqlancer::set_deadline(Some(Instant::now() + Duration::from_secs(5)));
    let before = coverage::snapshot().expect("an instrumented SQLite");
    if let Ok(db) = sqlancer::open(&input.0) {
        for q in &input.1 {
            let _ = sqlancer::run(&db, &q.sql());
        }
    }
    let delta = before.delta(&coverage::snapshot().expect("an instrumented SQLite"));
    let late = sqlancer::past_deadline();
    sqlancer::set_deadline(None);
    (!late).then(|| delta.hits().iter().map(|&(c, _)| c as u32).collect())
}

/// Exercise 100 fresh draws (sizes 0 to 99) and forget what they reached:
/// SQLite's once-per-process code (initialization, first allocations) then
/// runs here, so that it is not credited to the first inputs measured.
pub fn warm_up() {
    let gen = ephyra_sqlite::probes::case_queries();
    for i in 0..100 {
        exercise(&gen.generate(&mut Source::new(mix(0x0077_a2a0, i), i as usize)));
    }
}

/// SplitMix64 of a seed and an index.
pub fn mix(seed: u64, index: u64) -> u64 {
    let mut z = seed.wrapping_add(index.wrapping_mul(0x9e37_79b9_7f4a_7c15));
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// `qs` repaired against `case`'s final schema and kept within the row
/// budget, or drawn anew (counted as a fallback) when none is left.
pub fn queries_on(case: Case, qs: &[Query], g: &mut Source) -> (Case, Vec<Query>) {
    let schema = final_schema(&case);
    let q: Vec<Query> = repair_queries(&schema, qs).into_iter().filter(|q| q.fits(&schema)).collect();
    let q = if q.is_empty() {
        QUERY_FALLBACKS.fetch_add(1, Ordering::Relaxed);
        Queries(schema).generate(g)
    } else {
        q
    };
    (case, q)
}
