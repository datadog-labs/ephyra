// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! changes(), total_changes() and last_insert_rowid() after writes to a
//! virtual table.
use turso_apidiff_repro2::{exec, open, state};
fn main() {
    let c = open("/tmp/repro2-vtab-changes.db");
    exec(&c, "CREATE TABLE t(x)");
    exec(&c, "INSERT INTO t VALUES (1)");
    state(&c);
    exec(&c, "CREATE VIRTUAL TABLE v USING mem");
    state(&c);
    exec(&c, "INSERT INTO v(a, b) VALUES (1, 'a')");
    state(&c);
    exec(&c, "INSERT INTO v(a, b) VALUES (2, 'b')");
    state(&c);
    exec(&c, "UPDATE v SET a = a + 1");
    state(&c);
    exec(&c, "DELETE FROM v WHERE rowid = 1");
    state(&c);
    exec(&c, "SELECT changes(), total_changes(), last_insert_rowid()");
}
