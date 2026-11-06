// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Native virtual table: ALTER TABLE ... RENAME TO, then DROP TABLE of the
//! new name, on the connection that created it (one connection).
use turso_apidiff_repro3::*;

fn main() {
    let c = open("/tmp/r3-vtab-rename-drop.db");
    exec(&c, "CREATE VIRTUAL TABLE v USING mem");
    exec(&c, "INSERT INTO v(a, b) VALUES (1, 'x')");
    exec(&c, "ALTER TABLE v RENAME TO w");
    exec(&c, "SELECT rowid, a, b FROM w");
    exec(&c, "DROP TABLE w");
    exec(&c, "SELECT name FROM sqlite_schema");
    exec(&c, "INSERT INTO w(a, b) VALUES (2, 'y')");
    exec(&c, "SELECT rowid, a, b FROM w");
}
