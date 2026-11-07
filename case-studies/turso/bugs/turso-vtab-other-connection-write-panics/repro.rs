use std::sync::Arc;
use turso_core::{ConnectOpts, Connection, Database, OpenOptions, PlatformIO, StepResult};

// extensions are built with mimalloc (extensions/core/README.md); the host must use it too
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn exec(c: &Arc<Connection>, sql: &str) {
    let mut st = c.prepare(sql).unwrap();
    loop {
        match st.step() {
            Ok(StepResult::Row) => {
                let r: Vec<String> = st.row().unwrap().get_values().map(|v| v.to_string()).collect();
                println!("{}", r.join("|"));
            }
            Ok(StepResult::IO) => st._io().step().unwrap(),
            Ok(_) => break,
            Err(e) => { println!("error: {e}"); break; }
        }
    }
}

fn main() {
    let ext = "target/release/libturso_ext_tests.dylib";
    let io = Arc::new(PlatformIO::new().unwrap());
    let db = Database::open(io, "t.db", OpenOptions::new(Arc::new(turso_sqlite::SqliteDialect))).unwrap();
    let connect = || { let c = db.connect(ConnectOpts::new()).unwrap(); c.load_extension(ext).unwrap(); c };

    let a = connect();
    exec(&a, "CREATE VIRTUAL TABLE v USING kv_store");
    exec(&a, "INSERT INTO v(key, value) VALUES ('k1', 'x')");

    let b = connect();
    exec(&b, "SELECT key, value FROM v");
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        exec(&b, "INSERT INTO v(key, value) VALUES ('k2', 'y')")
    }));

    let c = connect();
    exec(&c, "DROP TABLE v");
    exec(&c, "SELECT name FROM sqlite_schema");
}
