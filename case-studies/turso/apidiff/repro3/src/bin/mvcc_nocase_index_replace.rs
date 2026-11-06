// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! MVCC: REPLACE (or UPDATE) changes an indexed COLLATE NOCASE column to a
//! value that compares equal under NOCASE ('B' -> 'b'); after the checkpoint
//! the index entry keeps the old value. Argument: the statement (default
//! `REPLACE INTO t1(id, a, b) VALUES (2, 2, 'b')`).
use turso_apidiff_repro3::*;

fn main() {
    let q = std::env::args().nth(1).unwrap_or_else(|| "REPLACE INTO t1(id, a, b) VALUES (2, 2, 'b')".into());
    let path = "/tmp/r3-mvcc-nocase.db";
    {
        let db = open_db_mvcc(path);
        let c = connect(&db);
        exec(&c, "CREATE TABLE t1(id INTEGER PRIMARY KEY AUTOINCREMENT, a INT, b TEXT COLLATE NOCASE, c REAL, d)");
        exec(&c, "CREATE INDEX t1_b ON t1(b)");
        exec(&c, "INSERT INTO t1(a, b) VALUES (0, 'x'), (2, 'B')");
        exec(&c, &q);
        exec(&c, "SELECT id, b FROM t1 ORDER BY id");
        exec(&c, "SELECT b, id FROM t1 INDEXED BY t1_b ORDER BY b");
        exec(&c, "PRAGMA integrity_check");
        exec(&c, "PRAGMA wal_checkpoint(TRUNCATE)");
        exec(&c, "SELECT b, id FROM t1 INDEXED BY t1_b ORDER BY b");
        exec(&c, "PRAGMA journal_mode = wal");
        exec(&c, "SELECT b, id FROM t1 INDEXED BY t1_b ORDER BY b");
        exec(&c, "PRAGMA integrity_check");
        let _ = c.close();
    }
    println!("-- reopened (WAL)");
    let opts = turso_core::OpenOptions::new(std::sync::Arc::new(turso_sqlite::SqliteDialect));
    let io = std::sync::Arc::new(turso_core::PlatformIO::new().unwrap());
    let db = turso_core::Database::open(io, path, opts).unwrap();
    let c = connect(&db);
    exec(&c, "SELECT id, b FROM t1 ORDER BY id");
    exec(&c, "SELECT b, id FROM t1 INDEXED BY t1_b ORDER BY b");
    exec(&c, "PRAGMA integrity_check");
}
