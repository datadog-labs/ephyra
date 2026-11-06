// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! A reader transaction gets "Database schema changed" after another
//! connection changes the schema of an attached database; the same change
//! on main is read from the transaction's snapshot.
use turso_apidiff_repro2::{connect, on, open_db_attach};
fn main() {
    let _ = std::fs::remove_file("/tmp/repro2-aux2.db");
    let _ = std::fs::remove_file("/tmp/repro2-aux2.db-wal");
    let db = open_db_attach("/tmp/repro2-main2.db");
    let (a, b) = (connect(&db), connect(&db));
    on("a", &a, "CREATE TABLE t(x)");
    on("a", &a, "INSERT INTO t VALUES (1)");
    on("a", &a, "ATTACH '/tmp/repro2-aux2.db' AS aux");
    on("b", &b, "ATTACH '/tmp/repro2-aux2.db' AS aux");
    on("a", &a, "CREATE TABLE aux.u(a)");
    on("a", &a, "INSERT INTO aux.u VALUES (1)");
    on("a", &a, "BEGIN");
    on("a", &a, "SELECT (SELECT count(*) FROM t), (SELECT count(*) FROM aux.u)");
    on("b", &b, "ALTER TABLE t ADD COLUMN e");
    on("a", &a, "SELECT * FROM t");
    on("b", &b, "ALTER TABLE aux.u ADD COLUMN e");
    on("a", &a, "SELECT * FROM aux.u");
    on("a", &a, "SELECT name FROM aux.sqlite_schema");
    on("a", &a, "COMMIT");
    on("a", &a, "SELECT * FROM aux.u");
}
