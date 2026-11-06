// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Issue repro: a virtual table from a turso_ext extension (../vtab-ext,
//! `memvt`, loaded with Connection::load_extension like `.load` in tursodb),
//! created on connection a, then written and dropped from other connections
//! of the same Database.
//! Usage: issue_vtab_other_connection [before|after] [path/to/extension.dylib] [module]
//! e.g. `after turso/target/release/libturso_ext_tests.dylib kv_store` (a build of Turso's extension tests)
//! for Turso's own test extension (extensions/tests; columns key, value).
use std::panic;
use std::sync::Arc;
use turso_apidiff_repro2::exec;
use turso_core::{ConnectOpts, Connection, Database, OpenOptions, PlatformIO};

// The extension allocates with mimalloc; the host must use the same allocator
// (extensions/core/README.md), as tursodb does.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn connect(db: &Arc<Database>, ext: &str) -> Arc<Connection> {
    let c = db.connect(ConnectOpts::new()).unwrap();
    c.load_extension(ext).unwrap();
    c
}

fn main() {
    let which = std::env::args().nth(1).unwrap_or_else(|| "before".into());
    let ext = std::env::args().nth(2).unwrap_or_else(|| {
        concat!(env!("CARGO_MANIFEST_DIR"), "/../vtab-ext/target/release/libmemvt.dylib").to_string()
    });
    let module = std::env::args().nth(3).unwrap_or_else(|| "memvt".into());
    let (c1, c2, v1, v2) = if module == "kv_store" { ("key", "value", "'k1', 'x'", "'k2', 'y'") } else { ("a", "b", "1, 'x'", "2, 'y'") };
    let path = format!("/tmp/issue-vtab-other-{which}.db");
    for s in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{path}{s}"));
    }
    let io = Arc::new(PlatformIO::new().unwrap());
    let db = Database::open(io, &path, OpenOptions::new(Arc::new(turso_sqlite::SqliteDialect))).unwrap();
    let a = connect(&db, &ext);
    // "before": b connects before the CREATE; "after": after it.
    let early = if which == "before" { Some(connect(&db, &ext)) } else { None };
    println!("-- connection a");
    exec(&a, &format!("CREATE VIRTUAL TABLE v USING {module}"));
    exec(&a, &format!("INSERT INTO v({c1}, {c2}) VALUES ({v1})"));
    let b = early.unwrap_or_else(|| connect(&db, &ext));
    println!("-- connection b (connected {which} the CREATE)");
    exec(&b, &format!("SELECT {c1}, {c2} FROM v"));
    let r = panic::catch_unwind(panic::AssertUnwindSafe(|| exec(&b, &format!("INSERT INTO v({c1}, {c2}) VALUES ({v2})"))));
    if r.is_err() {
        println!("  -> (panicked)");
    }
    let c = connect(&db, &ext);
    println!("-- connection c");
    exec(&c, "DROP TABLE v");
    exec(&c, "SELECT name FROM sqlite_schema");
    println!("-- connection a");
    exec(&a, "SELECT name FROM sqlite_schema");
    exec(&a, &format!("SELECT {c1}, {c2} FROM v"));
}
