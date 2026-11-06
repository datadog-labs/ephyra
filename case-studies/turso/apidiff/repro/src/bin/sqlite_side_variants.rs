// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! SQLite 3.50.4 side of the issue_* variant bins (issue_b_savepoint, issue_a_sibling, issue_a_commit, issue_f/g as written).
fn main() {
    println!("SQLite {}", rusqlite::version());
    // issue_b_savepoint
    {
        let dir = tempfile::tempdir().unwrap();
        let c = rusqlite::Connection::open(dir.path().join("t.db")).unwrap();
        c.execute_batch("CREATE TABLE t(x); INSERT INTO t VALUES (1), (2);").unwrap();
        let mut s = c.prepare("SELECT x FROM t").unwrap();
        {
            let mut rows = s.raw_query();
            println!("b_savepoint row: {:?}", rows.next().unwrap().unwrap().get_ref(0).unwrap());
            c.execute("SAVEPOINT sp", []).unwrap();
            c.execute("INSERT INTO t VALUES (3)", []).unwrap();
        }
        drop(s);
        println!("b_savepoint autocommit: {}", c.is_autocommit());
        println!("b_savepoint ROLLBACK TO: {:?}", c.execute("ROLLBACK TO sp", []));
        println!("b_savepoint RELEASE: {:?}", c.execute("RELEASE sp", []));
        let n: i64 = c.query_row("SELECT count(*) FROM t", [], |r| r.get(0)).unwrap();
        println!("b_savepoint count: {n}");
    }
    // issue_a_sibling / issue_a_commit
    for commit in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.db");
        let a = rusqlite::Connection::open(&p).unwrap();
        a.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE t(id INTEGER PRIMARY KEY, x); INSERT INTO t(x) VALUES (1), (2), (3);").unwrap();
        let b = rusqlite::Connection::open(&p).unwrap();
        let mut scan = b.prepare("SELECT id FROM t ORDER BY id").unwrap();
        let mut rows = scan.raw_query();
        println!("a(commit={commit}) scan: {:?}", rows.next().unwrap().unwrap().get_ref(0).unwrap());
        if commit {
            b.execute_batch("BEGIN; COMMIT;").unwrap();
        } else {
            let mut o = b.prepare("SELECT id FROM t WHERE id = 99").unwrap();
            println!("a other: {:?}", o.raw_query().next().unwrap().is_some());
        }
        a.execute("INSERT INTO t(x) VALUES (4)", []).unwrap();
        let n: i64 = b.query_row("SELECT count(*) FROM t", [], |r| r.get(0)).unwrap();
        println!("a(commit={commit}) count on B: {n}");
        while let Some(r) = rows.next().unwrap() {
            println!("a(commit={commit}) scan: {:?}", r.get_ref(0).unwrap());
        }
        println!("a(commit={commit}) scan done");
    }
    // issue_f: same order as issue_f_noop_ddl
    {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.db");
        let a = rusqlite::Connection::open(&p).unwrap();
        a.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE t(x); CREATE INDEX i ON t(x);").unwrap();
        let b = rusqlite::Connection::open(&p).unwrap();
        a.execute("BEGIN IMMEDIATE", []).unwrap();
        for q in ["CREATE TABLE IF NOT EXISTS t(x)", "CREATE INDEX IF NOT EXISTS i ON t(x)", "DROP TABLE IF EXISTS nosuch", "DROP INDEX IF EXISTS nosuch", "DELETE FROM t WHERE 0"] {
            println!("f {q}: {:?}", b.execute(q, []));
        }
    }
    // issue_panic_window: same order as the issue (window first prepared, then INSERT)
    {
        let c = rusqlite::Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE t(id INTEGER PRIMARY KEY, a); INSERT INTO t(a) VALUES (1), (2);").unwrap();
        let mut win = c.prepare("SELECT id, count(a) OVER (ORDER BY id) FROM t").unwrap();
        let mut win_rows = win.raw_query();
        println!("panic window row: {:?}", win_rows.next().unwrap().map(|r| r.get::<_, i64>(0).unwrap()));
        let mut ins = c.prepare("INSERT INTO t(a) VALUES (3) RETURNING id").unwrap();
        let mut ins_rows = ins.raw_query();
        println!("panic insert row: {:?}", ins_rows.next().unwrap().map(|r| r.get::<_, i64>(0).unwrap()));
        drop(win_rows);
        println!("panic reset: ok");
        println!("panic insert next: {:?}", ins_rows.next().map(|r| r.is_some()));
        drop(ins_rows);
        let n: i64 = c.query_row("SELECT count(*) FROM t", [], |r| r.get(0)).unwrap();
        println!("panic count: {n}");
    }
    // issue_e_reset: INSERT reset after its first row
    {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.db");
        let c = rusqlite::Connection::open(&p).unwrap();
        c.execute_batch("CREATE TABLE t(id INTEGER PRIMARY KEY, x); CREATE TABLE u(y);").unwrap();
        let mut sel = c.prepare("SELECT y FROM u").unwrap();
        c.execute("CREATE TABLE v(z)", []).unwrap();
        let mut ins = c.prepare("INSERT INTO t(x) VALUES (1) RETURNING id").unwrap();
        {
            let mut ins_rows = ins.raw_query();
            println!("e_reset insert row: {:?}", ins_rows.next().unwrap().map(|r| r.get::<_, i64>(0).unwrap()));
            let mut sel_rows = sel.raw_query();
            println!("e_reset select: {:?}", sel_rows.next().map(|r| r.is_some()));
        } // drops reset both
        let n: i64 = c.query_row("SELECT count(*) FROM t", [], |r| r.get(0)).unwrap();
        println!("e_reset count: {n}");
        let c2 = rusqlite::Connection::open(&p).unwrap();
        let n: i64 = c2.query_row("SELECT count(*) FROM t", [], |r| r.get(0)).unwrap();
        println!("e_reset count new conn: {n}");
    }
}
