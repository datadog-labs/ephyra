//! The property: SQLite running a case's statements on its file agrees
//! with the model, statement by statement.

use ephyra::{Gen, Source};
use ephyra_sqlite::{
    cases::{cases, model_of, Case, Cases},
    model::{Key, Value},
    semantics,
    statements::Statement,
};
use rusqlite::{types::ValueRef, Connection};

/// The file opened writable in memory. A header saying WAL is presented as
/// rollback mode, as the predicate does: with no `-wal` file, SQLite reads
/// the same pages.
fn open(bytes: &[u8]) -> Connection {
    let mut db = Connection::open_in_memory().unwrap();
    if !bytes.is_empty() {
        let mut bytes = bytes.to_vec();
        for at in [18, 19] {
            if bytes[at] == 2 {
                bytes[at] = 1;
            }
        }
        db.deserialize_read_exact(rusqlite::MAIN_DB, &bytes[..], bytes.len(), false).unwrap();
    }
    db
}

fn value(v: ValueRef) -> Value {
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => Value::Int(i),
        ValueRef::Real(x) => Value::Real(x),
        ValueRef::Text(t) => Value::Text(String::from_utf8(t.to_vec()).expect("UTF-8 text")),
        ValueRef::Blob(b) => Value::Blob(b.to_vec()),
    }
}

fn query(db: &Connection, sql: &str) -> rusqlite::Result<Vec<Vec<Value>>> {
    let mut stmt = db.prepare(sql)?;
    let n = stmt.column_count();
    let rows = stmt.query_map([], |r| (0..n).map(|k| r.get_ref(k).map(value)).collect::<Result<Vec<_>, _>>())?;
    rows.collect()
}

#[ephyra::test(gen = cases(), tests = 4000)]
fn sqlite_agrees_with_the_model(case: Case) {
    agree(&case);
}

/// Cases mutated as the runner's mutation profiles (`novel`, `rare`,
/// `near-miss`) mutate a corpus input, a chain of [`Cases`]' mutations
/// with the statements repaired after each, agree with SQLite too: the
/// draws above never mutate, so a statement that repair leaves invalid
/// (an UPDATE FROM whose filter names a column its target lost, run
/// 20261016-1828) only shows here.
#[test]
fn sqlite_agrees_with_the_model_on_mutated_cases() {
    let mut from = 0;
    for seed in 0..600u64 {
        let mut g = Source::new(seed ^ 0x6d75_7461, (seed % 101) as usize);
        let mut case = Cases.generate(&mut g);
        for step in 0..16 {
            case = Cases.mutate(&case, &mut g);
            let n = case.statements.iter().filter(|s| matches!(s, Statement::Update(u) if u.from.is_some())).count();
            from += n;
            if n == 0 {
                continue;
            }
            let r = std::panic::catch_unwind(|| agree(&case));
            assert!(r.is_ok(), "seed {seed}, mutation {step}: {case:?}");
        }
    }
    assert!(from > 500, "{from} UPDATE FROM statements in the mutated cases");
}

/// Every UPDATE FROM repaired against another case's final state (another
/// file and statements: its tables of other widths, rows or keys, the
/// FROM row often missing) is valid there, as SQLite runs it: the
/// statement's repair is what keeps a mutated case's UPDATE FROM valid,
/// here exercised directly ([`sqlite_agrees_with_the_model_on_mutated_cases`]
/// reaches it only now and then).
#[test]
fn update_from_repaired_against_another_state_agrees_with_sqlite() {
    let (mut tried, mut kept) = (0, 0);
    // 3500 seeds: a case's statements are a virtual table's now and then.
    for seed in 0..3500u64 {
        let a = Cases.generate(&mut Source::new(seed, (seed % 101) as usize));
        let b = Cases.generate(&mut Source::new(seed ^ 0x0b, ((seed + 50) % 101) as usize));
        let mut m = model_of(&b.file).expect("a well formed case");
        for s in &b.statements {
            semantics::apply(&mut m, s);
        }
        let repaired: Vec<Statement> = a
            .statements
            .iter()
            .filter(|s| matches!(s, Statement::Update(u) if u.from.is_some()))
            .filter_map(|s| {
                tried += 1;
                ephyra_sqlite::cases::repair(&m, s)
            })
            .filter(|s| semantics::valid(&m, s))
            .collect();
        for s in repaired {
            kept += usize::from(matches!(&s, Statement::Update(u) if u.from.is_some()));
            let case = Case { file: b.file.clone(), statements: b.statements.iter().cloned().chain(std::iter::once(s)).collect() };
            let r = std::panic::catch_unwind(|| agree(&case));
            assert!(r.is_ok(), "seed {seed}: {case:?}");
        }
    }
    assert!(tried > 400 && kept > 80, "{tried} UPDATE FROM statements repaired, {kept} kept as UPDATE FROM");
}

fn agree(case: &Case) {
    let mut m = model_of(&case.file).expect("a well formed case");
    let db = open(&case.file.0);
    for (k, s) in case.statements.iter().enumerate() {
        let sql = s.to_string();
        let effect = semantics::apply(&mut m, s);
        match effect.rows {
            Some(expected) => {
                let actual = query(&db, &sql).unwrap_or_else(|e| panic!("statement {k}: {sql}: {e}"));
                assert!(expected.matches(&actual), "statement {k}: {sql}\nexpected {expected:?}\nactual {actual:?}");
            },
            None => {
                db.execute_batch(&sql).unwrap_or_else(|e| panic!("statement {k}: {sql}: {e}"));
                let check: Vec<String> = db
                    .prepare("PRAGMA integrity_check")
                    .and_then(|mut st| st.query_map([], |r| r.get(0))?.collect())
                    .unwrap_or_else(|e| panic!("integrity_check after statement {k}: {e}"));
                assert_eq!(check, ["ok"], "after statement {k}: {sql}");
            },
        }
    }
    for (k, t) in m.tables.iter().enumerate() {
        // In key order: a scan may read a covering index instead.
        let sql = match &t.key {
            Key::WithoutRowid(pk) => {
                format!("SELECT * FROM t{k} ORDER BY {}", pk.iter().map(|c| format!("c{c}")).collect::<Vec<_>>().join(", "))
            },
            _ => format!("SELECT rowid, * FROM t{k} ORDER BY rowid"),
        };
        let actual = query(&db, &sql).unwrap();
        let expected = semantics::contents(t);
        assert!(semantics::same_rows(&actual, &expected), "table t{k} at the end\nexpected {expected:?}\nactual {actual:?}");
    }
    // The named constraints as the model has them, in the order of the
    // table's SQL (the checks' `k{j}` apart): what DROP CONSTRAINT finds.
    for (k, t) in m.tables.iter().enumerate() {
        let sql = format!("SELECT sql FROM {}.sqlite_schema WHERE type = 'table' AND name = 't{k}'", t.db.name());
        let Some(Value::Text(text)) = query(&db, &sql).unwrap().first().map(|r| r[0].clone()) else {
            panic!("no SQL of table t{k}");
        };
        let named: Vec<String> = text
            .split("CONSTRAINT ")
            .skip(1)
            .map(|rest| rest.split(' ').next().unwrap().trim_matches(|c| matches!(c, '"' | '[' | ']' | '`')).to_string())
            .filter(|n| !n.starts_with('k'))
            .collect();
        let expected: Vec<String> = t.names.iter().map(|n| n.name.clone()).collect();
        assert_eq!(named, expected, "the named constraints of t{k}: {text}");
    }
    // The virtual tables' rows as the model keeps them (an fts5vocab
    // table's are SQLite's).
    for (k, t) in m.vtabs.iter().enumerate() {
        if matches!(t.kind, ephyra_sqlite::vtabs::Kind::Vocab { .. }) {
            continue;
        }
        let actual = query(&db, &t.contents_sql(k)).unwrap();
        assert_eq!(actual, t.contents(), "virtual table vt{k} at the end");
    }
}

/// Virtual tables are drawn inside ordinary cases, filled with enough rows
/// for interior R-Tree nodes, and SQLite agrees with the model on them.
#[test]
fn virtual_tables_drawn_and_agree() {
    use ephyra_sqlite::vtabs::{Kind, VStmt};
    let (mut cases, mut kinds, mut big) = (0, std::collections::BTreeMap::new(), 0);
    for seed in 0..1500u64 {
        let case = Cases.generate(&mut Source::new(seed ^ 0x7674_6162, (seed % 101) as usize));
        let mut made = false;
        for s in &case.statements {
            if let Statement::Virtual { s, kind } = s {
                made = true;
                if let VStmt::Create { .. } = s {
                    let k = match kind {
                        Kind::Rtree { .. } => "rtree",
                        Kind::Geopoly { .. } => "geopoly",
                        Kind::Fts5 { .. } => "fts5",
                        Kind::Vocab { .. } => "vocab",
                    };
                    *kinds.entry(k).or_insert(0) += 1;
                }
                if let VStmt::Fill { count, .. } = s {
                    big += usize::from(*count >= 100);
                }
            }
        }
        if made {
            cases += 1;
            agree(&case);
        }
    }
    assert!(cases > 200, "{cases} cases with virtual tables");
    assert!(big > 20, "{big} large fills");
    for k in ["rtree", "geopoly", "fts5", "vocab"] {
        assert!(kinds.get(k).copied().unwrap_or(0) >= 10, "{k}: {kinds:?}");
    }
}

/// The storage statements are drawn inside ordinary cases, each kind
/// often enough, and agree with the model where drawn (journal modes
/// followed by rollbacks only where a journal is kept, BEGIN ended by
/// COMMIT or ROLLBACK, auto-vacuum modes followed by writes).
#[test]
fn storage_statements_drawn_and_agree() {
    use std::collections::BTreeMap;
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for seed in 0..1500u64 {
        let case = Cases.generate(&mut Source::new(seed ^ 0x5707_a6e5, (seed % 101) as usize));
        let mut storage = false;
        let mut off = false;
        for s in &case.statements {
            let k = match s {
                Statement::JournalMode(j) => format!("journal_mode {j:?}"),
                Statement::Begin(_) => "begin".into(),
                Statement::Commit => "commit".into(),
                Statement::AutoVacuum(_) => "auto_vacuum".into(),
                Statement::TempStore(_) => "temp_store".into(),
                Statement::MmapSize(_) => "mmap_size".into(),
                Statement::VacuumInto(_) => "vacuum_into".into(),
                Statement::Rollback | Statement::RollbackTo(_) => {
                    assert!(!off, "seed {seed}: a rollback after journal_mode OFF: {case:?}");
                    continue;
                },
                _ => continue,
            };
            // WAL is refused in memory: the mode stays OFF.
            if let Statement::JournalMode(j) = s {
                if *j != ephyra_sqlite::statements::Journal::Wal {
                    off = *j == ephyra_sqlite::statements::Journal::Off;
                }
            }
            storage = true;
            *seen.entry(k).or_default() += 1;
        }
        if storage {
            agree(&case);
        }
    }
    for k in ["journal_mode Delete", "journal_mode Persist", "journal_mode Off", "begin", "commit", "auto_vacuum", "temp_store", "mmap_size", "vacuum_into"] {
        assert!(seen.get(k).copied().unwrap_or(0) >= 10, "{k}: {seen:?}");
    }
}
