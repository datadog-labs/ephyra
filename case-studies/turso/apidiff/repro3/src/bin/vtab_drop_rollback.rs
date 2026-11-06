// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Native virtual table: DROP TABLE inside a transaction, ROLLBACK (or
//! ROLLBACK TO), then use the table again on the same connection.
//! Argument: `insert` (default), `rename`, or `savepoint-insert`.
use turso_apidiff_repro3::*;

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "insert".into());
    let c = open("/tmp/r3-vtab-drop-rollback.db");
    exec(&c, "CREATE VIRTUAL TABLE v USING mem");
    exec(&c, "INSERT INTO v(a, b) VALUES (1, 'x')");
    if mode.starts_with("savepoint") {
        exec(&c, "SAVEPOINT s1");
        exec(&c, "DROP TABLE v");
        exec(&c, "ROLLBACK TO s1");
        exec(&c, "RELEASE s1");
    } else {
        exec(&c, "BEGIN");
        exec(&c, "DROP TABLE v");
        exec(&c, "ROLLBACK");
    }
    exec(&c, "SELECT name FROM sqlite_schema");
    exec(&c, "SELECT rowid, a, b FROM v");
    if mode == "rename" {
        exec(&c, "ALTER TABLE v RENAME TO w");
    } else {
        exec(&c, "INSERT INTO v(a, b) VALUES (2, 'y')");
    }
    println!("(not reached when the statement above panics)");
}
