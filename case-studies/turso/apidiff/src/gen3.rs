// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Round-3 generator (README.md, "Generators"). Same action language
//! and script format as rounds 1 and 2; the round-2 weights (re-prepare,
//! windows, reset/finalize with two statements running) are kept, and on
//! top of them:
//! - ATTACH depth: a second attached database (`aux2.u1`, 40% of attached
//!   sessions), the temp database (TEMP tables and TEMP triggers on main
//!   tables, connection 0), DETACH/ATTACH as actions (also while statements
//!   are suspended and inside transactions: error parity), checkpoints of
//!   main and the attached databases (`PRAGMA aux.wal_checkpoint(MODE)`;
//!   rows not compared, see README.md), and cross-database statements
//!   (`INSERT INTO aux.u0 SELECT ... FROM main.t0`, joins, `UPDATE ...
//!   WHERE a IN (SELECT ... FROM other db)`) interleaved with suspended
//!   readers;
//! - MVCC sessions (12%): Turso opens the file in MVCC mode, SQLite stays in
//!   WAL. One connection: `BEGIN CONCURRENT` (SQLite: `BEGIN DEFERRED`)
//!   besides the usual transaction control. Two connections: plain
//!   transactions only (no ATTACH in MVCC sessions);
//! - virtual tables: RENAME (xRename / VirtualTable::rename) and back,
//!   DROP of a vtab inside BEGIN or SAVEPOINT followed by ROLLBACK [TO],
//!   a vtab scan suspended across a write to the same vtab, triggers on
//!   real tables that read (or write) a vtab.

use crate::{
    engines::Cell,
    gen::{lit, small_int, txn, Action, Rng, Session, Slot, STEP_ALL},
    gen2::{ddl2, read2, setup2, table, write2, S2},
};

struct S3 {
    s2: S2,
    aux2: bool,
    temp: bool,
    mvcc: bool,
}

fn setup3(r: &mut Rng, out: &mut Vec<String>, mvcc: bool) -> S3 {
    let mut s2 = setup2(r, out);
    // setup2 attached aux in 25% of the sessions; round 3 wants 35% (not
    // in MVCC sessions: an MVCC main with a WAL attachment is a mode mix
    // SQLite cannot mirror).
    if mvcc {
        // "Virtual tables are not supported in MVCC mode" (Turso).
        out.retain(|q| !q.starts_with("CREATE VIRTUAL TABLE") && !q.starts_with("INSERT INTO vt"));
        s2.vtabs.clear();
    }
    if mvcc && s2.attach {
        out.retain(|q| !q.contains("aux."));
        out.retain(|q| !q.starts_with("ATTACH"));
        s2.names.retain(|n| !n.starts_with("aux."));
        s2.attach = false;
    } else if !mvcc && !s2.attach && r.chance(0.13) {
        out.push(format!("ATTACH '{}' AS aux", crate::engines::AUX));
        out.push("CREATE TABLE aux.u0(id INTEGER PRIMARY KEY, a INT, b TEXT, c REAL, d)".into());
        let n = r.below(8);
        if n > 0 {
            let rows: Vec<String> = (0..n).map(|_| format!("({}, {}, {}, {})", small_int(r).sql(), lit(r), lit(r), lit(r))).collect();
            out.push(format!("INSERT INTO aux.u0(a, b, c, d) VALUES {}", rows.join(", ")));
        }
        s2.names.push("aux.u0".into());
        s2.attach = true;
    }
    let mut aux2 = false;
    if s2.attach && r.chance(0.4) {
        aux2 = true;
        out.push(format!("ATTACH '{}' AS aux2", crate::engines::AUX2));
        out.push("CREATE TABLE aux2.u1(id INTEGER PRIMARY KEY, a INT, b TEXT, c REAL, d)".into());
        if r.chance(0.5) {
            out.push("CREATE INDEX aux2.u1_a ON u1(a)".into());
        }
        out.push("INSERT INTO aux2.u1(a, b, c, d) SELECT a, b, c, d FROM aux.u0".into());
        s2.names.push("aux2.u1".into());
    }
    let mut temp = false;
    if r.chance(0.25) {
        temp = true;
        out.push("CREATE TEMP TABLE w0(id INTEGER PRIMARY KEY, a, b)".into());
        if r.chance(0.5) {
            out.push("INSERT INTO temp.w0(a, b) SELECT a, b FROM t0".into());
        }
        if r.chance(0.6) {
            out.push(match r.below(3) {
                0 => "CREATE TEMP TRIGGER trw AFTER INSERT ON t0 BEGIN INSERT INTO w0(a, b) VALUES (new.a, new.b); END".into(),
                1 => "CREATE TEMP TRIGGER trw AFTER DELETE ON t0 BEGIN INSERT INTO w0(a, b) VALUES (old.id, 'del'); END".into(),
                _ => "CREATE TEMP TRIGGER trw AFTER UPDATE ON t0 BEGIN UPDATE w0 SET b = new.b WHERE a = new.a; END".into(),
            });
        }
    }
    // Triggers on a real table that read (or, rarely, write) a vtab.
    if !s2.vtabs.is_empty() && r.chance(0.25) {
        let v = s2.vtabs[0].clone();
        // Writing a vtab from a trigger: Turso rejects it ("unsafe use of
        // virtual table", a skip), so rare.
        if r.chance(0.95) {
            out.push(format!("CREATE TRIGGER trv AFTER INSERT ON t0 BEGIN UPDATE t0 SET c = (SELECT count(*) FROM {v}) WHERE id = new.id; END"));
        } else {
            out.push(format!("CREATE TRIGGER trv AFTER DELETE ON t0 BEGIN INSERT INTO {v}(a, b) VALUES (old.a, 'trg'); END"));
        }
    }
    if mvcc {
        // Turso's MVCC rowid allocator never reuses a rowid (ignored, failed
        // or rolled-back inserts, deleted maximum: README.md, "Generators"); with
        // AUTOINCREMENT both engines follow the same sequence rule.
        for q in out.iter_mut() {
            if q.starts_with("CREATE TABLE") && q.contains("id INTEGER PRIMARY KEY,") {
                *q = q.replacen("id INTEGER PRIMARY KEY,", "id INTEGER PRIMARY KEY AUTOINCREMENT,", 1);
            }
        }
    }
    S3 { s2, aux2, temp, mvcc }
}

fn checkpoint(r: &mut Rng, s: &S3) -> String {
    // MVCC: only TRUNCATE (PASSIVE needs an experimental flag, manual.md).
    let mode = if s.mvcc { "(TRUNCATE)" } else { r.pick(&["", "(PASSIVE)", "(FULL)", "(RESTART)", "(TRUNCATE)"]) };
    let mut dbs = vec!["main.", ""];
    if s.s2.attach {
        dbs.extend(["aux.", "aux."]);
    }
    if s.aux2 {
        dbs.push("aux2.");
    }
    format!("PRAGMA {}wal_checkpoint{mode}", r.pick(&dbs))
}

/// Statements across databases (main/aux/aux2/temp).
fn cross(r: &mut Rng, s: &S3) -> (String, usize, bool) {
    let t = format!("t{}", r.below(s.s2.base.tables));
    let mut others: Vec<&str> = vec![];
    if s.s2.attach {
        others.push("aux.u0");
    }
    if s.aux2 {
        others.push("aux2.u1");
    }
    if s.temp {
        others.push("temp.w0");
    }
    if others.is_empty() {
        return (format!("SELECT count(*) FROM main.{t}"), 0, false);
    }
    let o = r.pick(&others).to_string();
    let k = r.below(3);
    let cols = if o == "temp.w0" { "a, b" } else { "a, b, c, d" };
    match r.below(9) {
        0 => (format!("INSERT INTO {o}({cols}) SELECT {cols} FROM main.{t} WHERE id % 3 = {k} ORDER BY id"), 0, true),
        1 => (format!("INSERT INTO main.{t}(a, b) SELECT a, b FROM {o} WHERE id % 2 = {} ORDER BY id", k % 2), 0, true),
        2 => (format!("UPDATE {o} SET a = a + 1 WHERE a IN (SELECT a FROM main.{t} WHERE id > ?1)"), 1, true),
        3 => (format!("DELETE FROM {o} WHERE id IN (SELECT id FROM main.{t} WHERE a > {k})"), 0, true),
        4 if others.len() > 1 => {
            let p = others.iter().find(|x| **x != o).unwrap().to_string();
            (format!("INSERT INTO {o}(a, b) SELECT a, b FROM {p} ORDER BY id LIMIT 3"), 0, true)
        },
        5 => (format!("SELECT x.id, y.id, x.b FROM main.{t} x JOIN {o} y ON y.a = x.a ORDER BY 1, 2"), 0, false),
        6 => (format!("SELECT (SELECT count(*) FROM main.{t}), (SELECT count(*) FROM {o}), (SELECT max(id) FROM {o})"), 0, false),
        7 => (format!("SELECT id, a FROM {o} WHERE a NOT IN (SELECT a FROM main.{t} WHERE a IS NOT NULL) ORDER BY id"), 0, false),
        _ => (format!("SELECT a, count(*) FROM (SELECT a FROM main.{t} UNION ALL SELECT a FROM {o}) GROUP BY a ORDER BY a"), 0, false),
    }
}

pub fn session_v3(seed: u64) -> Session {
    let mut r = Rng::new(seed);
    let mut setup_sql = Vec::new();
    let mvcc = r.chance(0.12);
    let mut schema = setup3(&mut r, &mut setup_sql, mvcc);
    let conns = if r.chance(if schema.s2.attach { 0.6 } else if mvcc { 0.4 } else { 0.3 }) { 2 } else { 1 };
    let nslots = 2 + r.below(4);
    let mut slots = vec![(Slot::Empty, 0usize, 0usize, false); nslots]; // state, conn, params, writer
    let mut savepoints = 0usize;
    let mut actions = Vec::new();
    let mut in_txn = [false; 2];
    let mut touched = [false; 2];
    // Attached on connection k (generator's belief): aux, aux2.
    let mut attached = [[schema.s2.attach, schema.aux2], [false, false]];
    if schema.s2.attach && conns == 2 {
        actions.push(Action::Exec { conn: 1, sql: format!("ATTACH '{}' AS aux", crate::engines::AUX) });
        attached[1][0] = true;
        if schema.aux2 && r.chance(0.5) {
            actions.push(Action::Exec { conn: 1, sql: format!("ATTACH '{}' AS aux2", crate::engines::AUX2) });
            attached[1][1] = true;
        }
    }
    // A renamed vtab: (index into vtabs, original name).
    let mut renamed: Option<(usize, String)> = None;
    let len = 6 + r.below(36);
    let note_txn = |sql: &str, conn: usize, in_txn: &mut [bool; 2]| {
        let u = sql.to_uppercase();
        if u.starts_with("BEGIN") || u.starts_with("SAVEPOINT") {
            in_txn[conn] = true;
        } else if u.starts_with("COMMIT") || u.starts_with("END") || (u.starts_with("ROLLBACK") && !u.starts_with("ROLLBACK TO")) {
            in_txn[conn] = false;
        }
    };
    let has_extra = schema.s2.attach || schema.temp;
    while actions.len() < len {
        let running = slots.iter().filter(|s| s.0 == Slot::Running).count();
        let running_writer = slots.iter().any(|s| s.0 == Slot::Running && s.3);
        let ready = slots.iter().any(|s| s.0 == Slot::Ready);
        let empty: Vec<usize> = (0..nslots).filter(|&k| slots[k].0 == Slot::Empty).collect();
        let live: Vec<usize> = (0..nslots).filter(|&k| slots[k].0 != Slot::Empty).collect();
        let ready_params: Vec<usize> = (0..nslots).filter(|&k| slots[k].0 == Slot::Ready && slots[k].2 > 0).collect();
        let conn = if conns == 2 && r.chance(0.3) { 1 } else { 0 };
        let w = [
            if empty.is_empty() { 0 } else { 4 },                                  // 0 prepare
            if live.is_empty() { 0 } else { 6 },                                   // 1 step
            if running > 0 { 6 } else { 3 },                                       // 2 exec txn
            if running > 0 && !running_writer { 4 } else { 2 },                    // 3 exec write
            if live.is_empty() { 0 } else if running >= 2 { 3 } else { 1 },        // 4 reset
            if live.is_empty() { 0 } else if running >= 2 { 2 } else { 1 },        // 5 finalize
            if ready_params.is_empty() { 0 } else { 3 },                           // 6 bind
            if running_writer { 3 } else { 0 },                                    // 7 finish the writer
            if ready || running > 0 { 3 } else { 1 },                              // 8 exec DDL (re-prepare)
            if has_extra { if running > 0 { 4 } else { 2 } } else { 0 },           // 9 cross-database exec
            if schema.s2.attach { if running > 0 { 3 } else { 1 } } else { 0 },    // 10 ATTACH/DETACH/checkpoint
            if schema.s2.vtabs.is_empty() { 0 } else { 2 },                        // 11 vtab rename/drop+rollback/suspended scan
            if !schema.s2.attach { 1 } else { 0 },                                 // 12 checkpoint of main (no attach)
        ];
        match r.weighted(&w) {
            0 => {
                let slot = *r.pick(&empty);
                let (sql, params, writer) = match r.weighted(&[10, 7, 2, 1, if has_extra { 4 } else { 0 }]) {
                    0 => {
                        let (s, p) = read2(&mut r, &schema.s2);
                        (s, p, false)
                    },
                    1 => {
                        let (s, p) = write2(&mut r, &schema.s2, conn == 0);
                        (s, p, true)
                    },
                    2 => (txn3(&mut r, &mut savepoints, &schema, conns), 0, false),
                    3 => (ddl2(&mut r, &schema.s2), 0, true),
                    _ => cross(&mut r, &schema),
                };
                actions.push(Action::Prepare { slot, conn, sql });
                slots[slot] = (Slot::Ready, conn, params, writer);
                if params > 0 && r.chance(0.8) {
                    for idx in 1..=params {
                        let val = if r.chance(0.5) { small_int(&mut r) } else if r.chance(0.2) { Cell::Int(13) } else { crate::gen::value(&mut r) };
                        actions.push(Action::Bind { slot, idx, val });
                    }
                }
            },
            1 => {
                let slot = *r.pick(&live);
                if slots[slot].0 == Slot::Halted && r.chance(0.5) {
                    actions.push(Action::Reset { slot });
                }
                let n = match r.weighted(&[6, 3, 2]) {
                    0 => 1,
                    1 => 2 + r.below(2),
                    _ => STEP_ALL,
                };
                actions.push(Action::Step { slot, n });
                slots[slot].0 = if n == STEP_ALL { Slot::Halted } else { Slot::Running };
            },
            2 => {
                let sql = txn3(&mut r, &mut savepoints, &schema, conns);
                if conns == 2 && (sql.starts_with("SAVEPOINT") || sql == "BEGIN") && !touched[conn] && r.chance(0.8) {
                    if !in_txn[conn] {
                        actions.push(Action::Exec { conn, sql: "BEGIN".into() });
                        in_txn[conn] = true;
                    }
                    let touch = if attached[conn][0] { "SELECT (SELECT count(*) FROM t0), (SELECT count(*) FROM aux.u0)" } else { "SELECT count(*) FROM t0" };
                    actions.push(Action::Exec { conn, sql: touch.into() });
                    touched[conn] = in_txn[conn];
                    if sql == "BEGIN" {
                        continue;
                    }
                }
                note_txn(&sql, conn, &mut in_txn);
                if !in_txn[conn] {
                    touched[conn] = false;
                }
                actions.push(Action::Exec { conn, sql });
            },
            3 => {
                let (s, p) = write2(&mut r, &schema.s2, conn == 0);
                let mut s = s;
                for idx in (1..=p).rev() {
                    s = s.replace(&format!("?{idx}"), &small_int(&mut r).sql());
                }
                actions.push(Action::Exec { conn, sql: s });
            },
            4 => {
                let slot = *r.pick(&live);
                actions.push(Action::Reset { slot });
                slots[slot].0 = Slot::Ready;
            },
            5 => {
                let slot = *r.pick(&live);
                actions.push(Action::Finalize { slot });
                slots[slot].0 = Slot::Empty;
            },
            6 => {
                let slot = *r.pick(&ready_params);
                let idx = 1 + r.below(slots[slot].2);
                let val = crate::gen::value(&mut r);
                actions.push(Action::Bind { slot, idx, val });
            },
            7 => {
                let ws: Vec<usize> = (0..nslots).filter(|&k| slots[k].0 == Slot::Running && slots[k].3).collect();
                let slot = *r.pick(&ws);
                actions.push(Action::Step { slot, n: STEP_ALL });
                slots[slot].0 = Slot::Halted;
            },
            8 => {
                let c = if conns == 2 && r.chance(0.5) { 1 } else { 0 };
                actions.push(Action::Exec { conn: c, sql: ddl2(&mut r, &schema.s2) });
            },
            9 => {
                // Temp objects exist on connection 0 only.
                let c = if schema.temp { 0 } else { conn };
                let (mut s, p, _) = cross(&mut r, &schema);
                for idx in (1..=p).rev() {
                    s = s.replace(&format!("?{idx}"), &small_int(&mut r).sql());
                }
                actions.push(Action::Exec { conn: c, sql: s });
            },
            10 => {
                let c = if conns == 2 && r.chance(0.25) { 1 } else { 0 };
                match r.weighted(&[3, 3, 4, 1]) {
                    // DETACH (whatever is suspended or open: error parity).
                    0 => {
                        let which = if schema.aux2 && r.chance(0.6) { 1 } else { 0 };
                        let name = ["aux", "aux2"][which];
                        actions.push(Action::Exec { conn: c, sql: format!("DETACH {name}") });
                        attached[c][which] = false;
                    },
                    // ATTACH again (also inside a transaction).
                    1 => {
                        let which = if schema.aux2 && r.chance(0.5) { 1 } else { 0 };
                        let (name, file) = [("aux", crate::engines::AUX), ("aux2", crate::engines::AUX2)][which];
                        actions.push(Action::Exec { conn: c, sql: format!("ATTACH '{file}' AS {name}") });
                        attached[c][which] = true;
                    },
                    2 => actions.push(Action::Exec { conn: c, sql: checkpoint(&mut r, &schema) }),
                    // DETACH and ATTACH back as a pair.
                    _ => {
                        actions.push(Action::Exec { conn: c, sql: "DETACH aux".into() });
                        actions.push(Action::Exec { conn: c, sql: format!("ATTACH '{}' AS aux", crate::engines::AUX) });
                        attached[c][0] = true;
                    },
                }
            },
            11 => {
                let i = r.below(schema.s2.vtabs.len());
                let v = schema.s2.vtabs[i].clone();
                match r.weighted(&[3, 3, 2, 3]) {
                    0 => {
                        // RENAME and back (xRename / VirtualTable::rename).
                        if let Some((k, orig)) = renamed.take() {
                            let cur = schema.s2.vtabs[k].clone();
                            actions.push(Action::Exec { conn: 0, sql: format!("ALTER TABLE {cur} RENAME TO {orig}") });
                            schema.s2.vtabs[k] = orig;
                        } else {
                            let to = format!("vt{}", 7 + i);
                            actions.push(Action::Exec { conn: 0, sql: format!("ALTER TABLE {v} RENAME TO {to}") });
                            schema.s2.vtabs[i] = to;
                            renamed = Some((i, v));
                        }
                    },
                    1 => {
                        // DROP of a vtab inside BEGIN or SAVEPOINT, then ROLLBACK [TO].
                        let sp = r.chance(0.4);
                        if sp {
                            savepoints += 1;
                            actions.push(Action::Exec { conn: 0, sql: format!("SAVEPOINT d{savepoints}") });
                        } else if !in_txn[0] {
                            actions.push(Action::Exec { conn: 0, sql: "BEGIN".into() });
                        }
                        actions.push(Action::Exec { conn: 0, sql: format!("DROP TABLE {v}") });
                        if r.chance(0.5) {
                            actions.push(Action::Exec { conn: 0, sql: format!("SELECT count(*) FROM {v}") });
                        }
                        if sp {
                            actions.push(Action::Exec { conn: 0, sql: format!("ROLLBACK TO d{savepoints}") });
                            if !in_txn[0] {
                                actions.push(Action::Exec { conn: 0, sql: format!("RELEASE d{savepoints}") });
                            }
                        } else {
                            actions.push(Action::Exec { conn: 0, sql: "ROLLBACK".into() });
                            in_txn[0] = false;
                            touched[0] = false;
                        }
                        actions.push(Action::Exec { conn: 0, sql: format!("SELECT rowid, a, b FROM {v} ORDER BY rowid") });
                    },
                    2 => {
                        // SAVEPOINT + a failing vtab write (known vt-rollback
                        // when Turso calls rollback) + ROLLBACK TO or RELEASE.
                        savepoints += 1;
                        actions.push(Action::Exec { conn: 0, sql: format!("SAVEPOINT f{savepoints}") });
                        actions.push(Action::Exec { conn: 0, sql: format!("INSERT INTO {v}(a, b) VALUES ({}, {})", small_int(&mut r).sql(), lit(&mut r)) });
                        actions.push(Action::Exec { conn: 0, sql: format!("INSERT INTO {v}(a, b) VALUES (1, 'boom')") });
                        let end = if r.chance(0.5) { format!("ROLLBACK TO f{savepoints}") } else { format!("RELEASE f{savepoints}") };
                        actions.push(Action::Exec { conn: 0, sql: end });
                        if !in_txn[0] && r.chance(0.7) {
                            actions.push(Action::Exec { conn: 0, sql: format!("RELEASE f{savepoints}") });
                        }
                    },
                    _ => {
                        // A vtab scan suspended across a write to the same vtab.
                        if let Some(&slot) = empty.first() {
                            actions.push(Action::Prepare { slot, conn: 0, sql: format!("SELECT rowid, a, b FROM {v} ORDER BY rowid") });
                            actions.push(Action::Step { slot, n: 1 });
                            let wsql = match r.below(3) {
                                0 => format!("INSERT INTO {v}(a, b) VALUES ({}, {})", small_int(&mut r).sql(), lit(&mut r)),
                                1 => format!("DELETE FROM {v} WHERE rowid = {}", 1 + r.below(4)),
                                _ => format!("UPDATE {v} SET a = a + 1 WHERE rowid = {}", 1 + r.below(4)),
                            };
                            actions.push(Action::Exec { conn: 0, sql: wsql });
                            actions.push(Action::Step { slot, n: STEP_ALL });
                            slots[slot] = (Slot::Halted, 0, 0, false);
                        }
                    },
                }
            },
            _ => actions.push(Action::Exec { conn, sql: checkpoint(&mut r, &schema) }),
        }
    }
    if schema.mvcc {
        // MVCC: a failed INSERT OR FAIL consumes an AUTOINCREMENT value
        // (gaps are permitted by SQLite's autoinc.html; README.md).
        for a in actions.iter_mut() {
            if let Action::Prepare { sql, .. } | Action::Exec { sql, .. } = a {
                *sql = sql.replace(" OR FAIL ", " OR ABORT ");
            }
        }
    }
    Session { seed, conns, mvcc: schema.mvcc, setup: setup_sql, actions }
}

/// Transaction control; in single-connection MVCC sessions a third of the
/// BEGINs are BEGIN CONCURRENT.
fn txn3(r: &mut Rng, savepoints: &mut usize, s: &S3, conns: usize) -> String {
    let t = txn(r, savepoints);
    if s.mvcc && conns == 1 && t.starts_with("BEGIN") && r.chance(0.35) {
        return "BEGIN CONCURRENT".into();
    }
    let _ = table;
    t
}
