// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! MVCC: SAVEPOINT; a SELECT stepped once; RELEASE (ends the transaction
//! while the SELECT is suspended); an autocommit write; step the SELECT.
//! Argument: `release` (default) or `commit` (BEGIN ... COMMIT instead), and
//! a second argument: `nowrite` skips the write, anything else is the write
//! statement (default: REPLACE INTO t(id, a, b) VALUES (2, 5, 'r')).
use turso_apidiff_repro3::*;

fn main() {
    let how = std::env::args().nth(1).unwrap_or_else(|| "release".into());
    let w = std::env::args().nth(2).unwrap_or_else(|| "REPLACE INTO t(id, a, b) VALUES (2, 5, 'r')".into());
    let db = open_db_mvcc("/tmp/r3-mvcc-release.db");
    let c = connect(&db);
    exec(&c, "CREATE TABLE t(id INTEGER PRIMARY KEY AUTOINCREMENT, a, b)");
    exec(&c, "INSERT INTO t(a, b) VALUES (1, 'x'), (2, 'y'), (3, 'z')");
    exec(&c, if how == "commit" { "BEGIN" } else { "SAVEPOINT s0" });
    println!("prepare s: SELECT id, a FROM t ORDER BY id");
    let mut s = c.prepare("SELECT id, a FROM t ORDER BY id").unwrap();
    step_once("s", &mut s);
    exec(&c, if how == "commit" { "COMMIT" } else { "RELEASE s0" });
    if w != "nowrite" {
        exec(&c, &w);
    }
    step_once("s", &mut s);
    step_once("s", &mut s);
    step_once("s", &mut s);
}
