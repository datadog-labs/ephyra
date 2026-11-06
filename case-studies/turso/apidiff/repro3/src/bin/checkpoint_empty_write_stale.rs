// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! WAL: after PRAGMA wal_checkpoint(RESTART|TRUNCATE), a write transaction
//! that writes nothing on connection a makes connection b's open read
//! transaction stale: b's next write fails "Database snapshot is stale".
//! Argument: the middle statement kind: `empty-txn` (default: BEGIN
//! IMMEDIATE; COMMIT), `noop-update`, `noop-ddl`, `none`, or `real-update`.
use turso_apidiff_repro3::*;

fn main() {
    let mid = std::env::args().nth(1).unwrap_or_else(|| "empty-txn".into());
    let mode = std::env::args().nth(2).unwrap_or_else(|| "TRUNCATE".into());
    let db = open_db("/tmp/r3-ckpt-stale.db");
    let a = connect(&db);
    let b = connect(&db);
    on("a", &a, "CREATE TABLE t(id INTEGER PRIMARY KEY, x)");
    on("a", &a, "INSERT INTO t(x) VALUES (1), (2)");
    on("a", &a, &format!("PRAGMA wal_checkpoint({mode})"));
    on("b", &b, "BEGIN");
    on("b", &b, "SELECT count(*) FROM t");
    match mid.as_str() {
        "empty-txn" => {
            on("a", &a, "BEGIN IMMEDIATE");
            on("a", &a, "COMMIT");
        },
        "noop-update" => on("a", &a, "UPDATE t SET x = 0 WHERE id = -5"),
        "noop-ddl" => on("a", &a, "CREATE TABLE IF NOT EXISTS t(y)"),
        "real-update" => on("a", &a, "UPDATE t SET x = 0 WHERE id = 1"),
        _ => {},
    }
    on("b", &b, "DELETE FROM t WHERE id = -1");
    on("b", &b, "COMMIT");
}
