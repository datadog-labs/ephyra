// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! MVCC, one connection: BEGIN CONCURRENT; a write; ALTER TABLE ADD COLUMN
//! (manual.md: schema changes upgrade a concurrent transaction to exclusive
//! in place). Arguments: the write (default `INSERT INTO t(a) VALUES (5)`,
//! `none` for no write) and the DDL (default `ALTER TABLE t ADD COLUMN e DEFAULT 1`).
use turso_apidiff_repro3::*;

fn main() {
    let w = std::env::args().nth(1).unwrap_or_else(|| "INSERT INTO t(a) VALUES (5)".into());
    let ddl = std::env::args().nth(2).unwrap_or_else(|| "ALTER TABLE t ADD COLUMN e DEFAULT 1".into());
    let db = open_db_mvcc("/tmp/r3-mvcc-ddl.db");
    let c = connect(&db);
    // APIDIFF_PLAIN_ROWID=1: no AUTOINCREMENT.
    let auto = if std::env::var_os("APIDIFF_PLAIN_ROWID").is_some() { "" } else { " AUTOINCREMENT" };
    exec(&c, &format!("CREATE TABLE t(id INTEGER PRIMARY KEY{auto}, a)"));
    exec(&c, "INSERT INTO t(a) VALUES (1), (2)");
    exec(&c, "BEGIN CONCURRENT");
    if w != "none" {
        exec(&c, &w);
    }
    exec(&c, &ddl);
    exec(&c, "COMMIT");
    exec(&c, "SELECT * FROM t ORDER BY id");
}
