//! A statement that fails inside BEGIN makes Turso call the virtual
//! table's rollback() while the transaction stays open: the virtual
//! table's earlier write in the same transaction is undone, the rest of
//! the transaction commits.
use turso_apidiff_repro2::{exec, open};
fn main() {
    let c = open("/tmp/repro2-vtab-rollback.db");
    exec(&c, "CREATE TABLE t(a NOT NULL)");
    exec(&c, "CREATE VIRTUAL TABLE v USING mem");
    exec(&c, "INSERT INTO v(a, b) VALUES (7, 'x')");
    exec(&c, "BEGIN");
    exec(&c, "UPDATE v SET a = 8 WHERE rowid = 1");
    exec(&c, "INSERT INTO t VALUES (NULL)");
    exec(&c, "INSERT INTO t VALUES (1)");
    exec(&c, "COMMIT");
    exec(&c, "SELECT rowid, a, b FROM v");
    exec(&c, "SELECT a FROM t");
}
