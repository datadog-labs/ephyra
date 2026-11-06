//! Inside BEGIN, DDL on an attached database fails with "Database snapshot
//! is stale" when only the main database changed since the transaction's
//! first read. An INSERT on the same attached table succeeds.
use turso_apidiff_repro2::{connect, on, open_db_attach};
fn main() {
    let _ = std::fs::remove_file("/tmp/repro2-aux1.db");
    let _ = std::fs::remove_file("/tmp/repro2-aux1.db-wal");
    let db = open_db_attach("/tmp/repro2-main1.db");
    let (a, b) = (connect(&db), connect(&db));
    on("a", &a, "CREATE TABLE t(x)");
    on("a", &a, "ATTACH '/tmp/repro2-aux1.db' AS aux");
    on("b", &b, "ATTACH '/tmp/repro2-aux1.db' AS aux");
    on("a", &a, "CREATE TABLE aux.u(a)");
    on("a", &a, "INSERT INTO aux.u VALUES (1)");
    on("b", &b, "BEGIN");
    on("b", &b, "SELECT (SELECT count(*) FROM t), (SELECT count(*) FROM aux.u)");
    on("a", &a, "INSERT INTO t VALUES (1)");
    on("b", &b, "INSERT INTO aux.u VALUES (2)");
    on("b", &b, "ALTER TABLE aux.u ADD COLUMN e");
    on("b", &b, "CREATE INDEX aux.i ON u(a)");
    on("b", &b, "ROLLBACK");
    println!("-- the DDL holds main's write lock too:");
    on("b", &b, "BEGIN");
    on("b", &b, "SELECT (SELECT count(*) FROM t), (SELECT count(*) FROM aux.u)");
    on("b", &b, "ALTER TABLE aux.u ADD COLUMN e");
    on("a", &a, "INSERT INTO t VALUES (2)");
    on("b", &b, "COMMIT");
}
