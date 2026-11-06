//! A statement that got Busy on an attached database is stepped again
//! after the other connection committed: Turso returns Busy once more
//! (the next step succeeds). The same steps on a main-database table
//! succeed at the first retry.
use std::sync::Arc;
use turso_apidiff_repro2::{connect, on, open_db_attach};
use turso_core::{Connection, StepResult};

fn step_print(name: &str, st: &mut turso_core::Statement) {
    loop {
        match st.step() {
            Ok(StepResult::IO) => st._io().step().unwrap(),
            r => {
                println!("{name}: step -> {:?}", r.map(|x| format!("{x:?}")));
                return;
            },
        }
    }
}

fn run(table: &str, a: &Arc<Connection>, b: &Arc<Connection>) {
    println!("--- table {table}");
    on("a", a, "BEGIN");
    on("a", a, &format!("UPDATE {table} SET x = x + 1"));
    let mut st = b.prepare(format!("INSERT INTO {table} VALUES (9)")).unwrap();
    step_print("b (INSERT, a holds the write lock)", &mut st);
    on("a", a, "COMMIT");
    step_print("b (retry after a's COMMIT)", &mut st);
    step_print("b (retry 2)", &mut st);
    step_print("b (retry 3)", &mut st);
    st.reset().unwrap();
    step_print("b (after reset)", &mut st);
    drop(st);
    on("b", b, &format!("INSERT INTO {table} VALUES (10)"));
    on("b", b, &format!("SELECT count(*) FROM {table}"));
}

fn main() {
    let _ = std::fs::remove_file("/tmp/repro2-aux3.db");
    let _ = std::fs::remove_file("/tmp/repro2-aux3.db-wal");
    let db = open_db_attach("/tmp/repro2-main3.db");
    let (a, b) = (connect(&db), connect(&db));
    on("a", &a, "CREATE TABLE t(x)");
    on("a", &a, "INSERT INTO t VALUES (1)");
    on("a", &a, "ATTACH '/tmp/repro2-aux3.db' AS aux");
    on("b", &b, "ATTACH '/tmp/repro2-aux3.db' AS aux");
    on("a", &a, "CREATE TABLE aux.u(x)");
    on("a", &a, "INSERT INTO aux.u VALUES (1)");
    run("t", &a, &b);
    run("aux.u", &a, &b);
}
