//! UPDATE of a virtual table's rowid panics.
use turso_apidiff_repro2::{exec, open};
fn main() {
    let c = open("/tmp/repro2-vtab-rowid.db");
    exec(&c, "CREATE VIRTUAL TABLE v USING mem");
    exec(&c, "INSERT INTO v(a, b) VALUES (7, 'x')");
    exec(&c, "UPDATE v SET rowid = rowid + 1 WHERE rowid = 1");
    exec(&c, "SELECT rowid, a, b FROM v");
}
