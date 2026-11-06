// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! MVCC: BEGIN [CONCURRENT]; a SELECT stepped once; COMMIT; an autocommit
//! write; drop the SELECT; then PRAGMA wal_checkpoint(TRUNCATE) and
//! PRAGMA journal_mode = wal on the same connection.
//! Arguments: BEGIN form (default `BEGIN CONCURRENT`; `none`: no BEGIN/COMMIT, autocommit SELECT), the write (default
//! `UPDATE t SET a = 99 WHERE id = 6`, which matches no row), or `nowrite`.
use turso_apidiff_repro3::*;

fn main() {
    let begin = std::env::args().nth(1).unwrap_or_else(|| "BEGIN CONCURRENT".into());
    let w = std::env::args().nth(2).unwrap_or_else(|| "UPDATE t SET a = 99 WHERE id = 6".into());
    let db = open_db_mvcc("/tmp/r3-mvcc-ckpt.db");
    let c = connect(&db);
    exec(&c, "CREATE TABLE t(id INTEGER PRIMARY KEY AUTOINCREMENT, a, b)");
    exec(&c, "INSERT INTO t(a, b) VALUES (1, 'x'), (2, 'y'), (3, 'z')");
    if begin != "none" {
        exec(&c, &begin);
    }
    println!("prepare s: SELECT id, a FROM t ORDER BY id");
    let mut s = c.prepare("SELECT id, a FROM t ORDER BY id").unwrap();
    step_once("s", &mut s);
    if begin != "none" {
        exec(&c, "COMMIT");
    }
    if w != "nowrite" {
        exec(&c, &w);
    }
    println!("drop s");
    drop(s);
    println!("autocommit: {}", c.get_auto_commit());
    exec(&c, "PRAGMA wal_checkpoint(TRUNCATE)");
    exec(&c, "PRAGMA journal_mode = wal");
    exec(&c, "SELECT id, a FROM t ORDER BY id");
    let c2 = connect(&db);
    on("c2", &c2, "SELECT id, a FROM t ORDER BY id");
    on("c2", &c2, "INSERT INTO t(a, b) VALUES (4, 'w')");
}
