//! A virtual table created on one connection, written or dropped from
//! another connection of the same Database.
use std::panic;
use turso_apidiff_repro2::{connect, exec, open_db};
fn main() {
    let which = std::env::args().nth(1).unwrap_or_else(|| "before".into());
    let db = open_db(&format!("/tmp/repro2-vtab-other-{which}.db"));
    let a = connect(&db);
    // "before": b connects before the CREATE; "after": after it.
    let early = if which == "before" { Some(connect(&db)) } else { None };
    exec(&a, "CREATE VIRTUAL TABLE v USING mem");
    exec(&a, "INSERT INTO v(a, b) VALUES (1, 'x')");
    let b = early.unwrap_or_else(|| connect(&db));
    println!("-- connection b ({which} the CREATE)");
    exec(&b, "SELECT rowid, a, b FROM v");
    let r = panic::catch_unwind(panic::AssertUnwindSafe(|| exec(&b, "INSERT INTO v(a, b) VALUES (2, 'y')")));
    if r.is_err() {
        println!("  -> (panicked)");
    }
    let b2 = connect(&db);
    println!("-- connection c (new)");
    exec(&b2, "DROP TABLE v");
    exec(&b2, "SELECT name FROM sqlite_schema");
}
