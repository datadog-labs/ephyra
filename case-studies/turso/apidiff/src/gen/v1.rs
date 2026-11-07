// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Round-1 generator (README.md, "Generators").
//! A schema and data (setup, run on connection 0), then API
//! actions over a few statement slots on one or two connections. The
//! generator is biased toward the patterns that found SQLite bugs in the
//! VDBE-direct campaign: statements stepped partway and left suspended
//! while another statement runs BEGIN/COMMIT/ROLLBACK/SAVEPOINT/RELEASE/
//! ROLLBACK TO or writes; RETURNING statements stepped partway; failing
//! statements (constraints, OR ROLLBACK/ABORT/FAIL); application functions.
//!
//! Its building blocks (schema setup, reads, writes, conflict clauses,
//! RETURNING, transaction control, DDL) are reused by [`super::v2`] and
//! [`super::v3`].

use super::{lit, small_int, value, Action, Rng, Session, Slot, STEP_ALL};
use crate::engines::Cell;

pub(crate) struct Schema {
    pub(crate) tables: usize,
    pub(crate) fk: bool,
    pub(crate) triggers: bool,
}

pub(crate) fn setup(r: &mut Rng, out: &mut Vec<String>) -> Schema {
    let tables = 1 + r.below(3);
    let fk = tables > 1 && r.chance(0.2);
    if fk {
        out.push("PRAGMA foreign_keys = ON".into());
    }
    for t in 0..tables {
        let atype = r.pick(&["INTEGER", "INT", "", "NUMERIC", "TEXT", "REAL"]);
        let mut acons = String::new();
        if r.chance(0.3) {
            acons.push_str(" UNIQUE");
        }
        if r.chance(0.2) {
            acons.push_str(" NOT NULL DEFAULT 0");
        }
        if r.chance(0.2) {
            acons.push_str(" CHECK (a <> 13)");
        }
        let mut bcons = String::new();
        if r.chance(0.1) {
            bcons.push_str(" COLLATE NOCASE");
        }
        if r.chance(0.15) {
            bcons.push_str(" UNIQUE");
        }
        let dtype = r.pick(&["BLOB", "ANY", ""]);
        let pk = if r.chance(0.15) { "INTEGER PRIMARY KEY AUTOINCREMENT" } else { "INTEGER PRIMARY KEY" };
        let fkcol = if fk && t > 0 {
            format!(", p INTEGER REFERENCES t0(id) ON DELETE {}", r.pick(&["CASCADE", "SET NULL", "RESTRICT", "NO ACTION"]))
        } else {
            String::new()
        };
        out.push(format!("CREATE TABLE t{t}(id {pk}, a {atype}{acons}, b TEXT{bcons}, c REAL, d {dtype}{fkcol})"));
        if r.chance(0.4) {
            out.push(format!("CREATE INDEX t{t}_b ON t{t}(b)"));
        }
        if r.chance(0.3) {
            out.push(format!("CREATE INDEX t{t}_ac ON t{t}(a, c)"));
        }
        if r.chance(0.15) {
            out.push(format!("CREATE INDEX t{t}_pa ON t{t}(a) WHERE a > 0"));
        }
        if r.chance(0.1) {
            out.push(format!("CREATE INDEX t{t}_ea ON t{t}(lower(b))"));
        }
    }
    let mut triggers = false;
    if r.chance(0.25) {
        triggers = true;
        let t = r.below(tables);
        let u = r.below(tables);
        out.push(match r.below(4) {
            0 => format!("CREATE TRIGGER tr{t} AFTER INSERT ON t{t} BEGIN UPDATE t{u} SET c = coalesce(c, 0) + 1 WHERE id = new.a; END"),
            1 if t != u => format!("CREATE TRIGGER tr{t} AFTER DELETE ON t{t} BEGIN INSERT INTO t{u}(a, b) VALUES (old.a, 'del'); END"),
            2 => format!("CREATE TRIGGER tr{t} BEFORE UPDATE ON t{t} WHEN new.a = 99 BEGIN SELECT RAISE(ABORT, 'no 99'); END"),
            _ => format!("CREATE TRIGGER tr{t} AFTER UPDATE OF a ON t{t} BEGIN UPDATE t{t} SET b = app_text(new.a) WHERE id = new.id AND b IS NOT app_text(new.a); END"),
        });
    }
    for t in 0..tables {
        let n = r.below(10);
        if n == 0 {
            continue;
        }
        let rows: Vec<String> = (0..n)
            .map(|_| {
                let a = if r.chance(0.6) { small_int(r).sql() } else { lit(r) };
                let p = if fk && t > 0 { format!(", {}", if r.chance(0.7) { (1 + r.below(4)).to_string() } else { "NULL".into() }) } else { String::new() };
                format!("({a}, {}, {}, {}{p})", lit(r), lit(r), lit(r))
            })
            .collect();
        let pcol = if fk && t > 0 { ", p" } else { "" };
        out.push(format!("INSERT OR IGNORE INTO t{t}(a, b, c, d{pcol}) VALUES {}", rows.join(", ")));
    }
    Schema { tables, fk, triggers }
}

/// A statement: SQL and its parameter count.
pub(crate) fn read(r: &mut Rng, s: &Schema) -> (String, usize) {
    let t = format!("t{}", r.below(s.tables));
    let u = format!("t{}", r.below(s.tables));
    match r.below(15) {
        0 => (format!("SELECT * FROM {t} ORDER BY id"), 0),
        1 => (format!("SELECT id, a, app_text(b), app_echo(c), app_type(d) FROM {t} WHERE a > ?1 ORDER BY id"), 1),
        2 => (format!("SELECT a, count(*), app_count(b), app_isum(a), sum(id) FROM {t} GROUP BY a ORDER BY a"), 0),
        3 => (format!("SELECT id, count(b) OVER (ORDER BY id), sum(id) OVER (ORDER BY id), app_echo(a) FROM {t} ORDER BY id"), 0),
        4 => (format!("SELECT x.id, y.id, app_concat(x.b, y.b) FROM {t} x JOIN {u} y ON x.a = y.a ORDER BY 1, 2"), 0),
        5 => ("SELECT key, value, type FROM json_each(?1) ORDER BY key".into(), 1),
        6 => (format!("SELECT id, b FROM {t} WHERE b IN (SELECT b FROM {u}) ORDER BY id"), 0),
        7 => (format!("SELECT id, a, app_echo(a) FROM {t} WHERE id = ?1"), 1),
        8 => (format!("SELECT count(*), max(id), app_isum(id), app_count(a) FROM {t}"), 0),
        9 => (format!("SELECT b, id FROM {t} ORDER BY b, id"), 0),
        10 => (format!("SELECT id, app_echo(a) FROM {t} WHERE app_echo(a) > ?1 ORDER BY id"), 1),
        11 => (format!("SELECT * FROM {t} WHERE a BETWEEN ?1 AND ?2 ORDER BY id"), 2),
        12 => (format!("SELECT j.key, j.value FROM {t}, json_each(json_array({t}.a, {t}.b)) j ORDER BY {t}.id, j.key"), 0),
        13 => (format!("WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 6) SELECT n, (SELECT count(*) FROM {t} WHERE a = n) FROM r ORDER BY n"), 0),
        _ => (format!("SELECT id, app_text(a), app_type(app_echo(a)) FROM {t} ORDER BY id"), 0),
    }
}

pub(crate) fn conflict(r: &mut Rng) -> &'static str {
    *r.pick(&["", "", "", " OR ROLLBACK", " OR ABORT", " OR FAIL", " OR IGNORE", " OR REPLACE"])
}

pub(crate) fn returning(r: &mut Rng, cols: &str) -> String {
    if r.chance(0.5) {
        format!(" RETURNING {cols}")
    } else {
        String::new()
    }
}

pub(crate) fn write(r: &mut Rng, s: &Schema) -> (String, usize) {
    let t = format!("t{}", r.below(s.tables));
    let u = format!("t{}", r.below(s.tables));
    match r.below(11) {
        0 => {
            let n = 1 + r.below(3);
            let rows: Vec<String> = (0..n).map(|_| format!("({}, {}, {}, {})", small_int(r).sql(), lit(r), lit(r), lit(r))).collect();
            let ret = returning(r, "id, a, app_text(b)");
            (format!("INSERT{} INTO {t}(a, b, c, d) VALUES {}{ret}", conflict(r), rows.join(", ")), 0)
        },
        1 => {
            let ret = returning(r, "*");
            (format!("INSERT{} INTO {t}(a, b, c) SELECT a + {}, b, c FROM {u} WHERE id % 2 = {} ORDER BY id{ret}", conflict(r), r.below(3), r.below(2)), 0)
        },
        2 => {
            let ret = returning(r, "id, a, b");
            (format!("UPDATE{} {t} SET a = a + 1, b = ?1 WHERE a > ?2{ret}", conflict(r)), 2)
        },
        3 => {
            let cond = r.pick(&["id % 2 = 0", "a IS NULL", "a > 2", "b = 'a'", "id = ?1", "app_type(d) = 4"]).to_string();
            let ret = returning(r, "id, app_echo(c)");
            let p = usize::from(cond.contains("?1"));
            (format!("DELETE FROM {t} WHERE {cond}{ret}"), p)
        },
        4 => {
            let ret = returning(r, "*");
            (format!("REPLACE INTO {t}(id, a, b) VALUES ({}, {}, {}){ret}", 1 + r.below(8), small_int(r).sql(), lit(r)), 0)
        },
        5 => {
            let ret = returning(r, "id, a, b");
            (format!("INSERT INTO {t}(id, a, b) VALUES ({}, {}, {}) ON CONFLICT(id) DO UPDATE SET a = excluded.a, b = b || 'u'{ret}", 1 + r.below(8), small_int(r).sql(), lit(r)), 0)
        },
        6 => (format!("UPDATE {t} SET c = (SELECT count(*) FROM {u}) WHERE id = ?1"), 1),
        7 => {
            let ret = returning(r, "id, app_echo(a), app_type(b)");
            (format!("INSERT{} INTO {t}(a, b) VALUES (?1, ?2){ret}", conflict(r)), 2)
        },
        8 => (format!("DELETE FROM {t} WHERE id IN (SELECT id FROM {t} ORDER BY id LIMIT 2)"), 0),
        9 => {
            let ret = returning(r, "id, a");
            (format!("UPDATE{} {t} SET a = {} WHERE id = {}{ret}", conflict(r), r.pick(&["NULL", "13", "99", "a * 2", "-a", "app_echo(b)"]), 1 + r.below(8)), 0)
        },
        _ => {
            let ret = returning(r, "id, a");
            (format!("UPDATE{} {t} SET a = a - 1 WHERE a IN (SELECT a FROM {u}){ret}", conflict(r)), 0)
        },
    }
}

pub(crate) fn txn(r: &mut Rng, savepoints: &mut usize) -> String {
    match r.weighted(&[4, 1, 4, 1, 3, 4, 2, 3]) {
        0 => "BEGIN".into(),
        1 => r.pick(&["BEGIN IMMEDIATE", "BEGIN EXCLUSIVE", "BEGIN DEFERRED"]).to_string(),
        2 => "COMMIT".into(),
        3 => "END".into(),
        4 => "ROLLBACK".into(),
        5 => {
            *savepoints += 1;
            format!("SAVEPOINT s{}", r.below(*savepoints))
        },
        6 => format!("RELEASE s{}", r.below((*savepoints).max(1))),
        _ => format!("ROLLBACK TO s{}", r.below((*savepoints).max(1))),
    }
}

pub(crate) fn ddl(r: &mut Rng, s: &Schema) -> String {
    let t = r.below(s.tables);
    let k = r.below(3);
    match r.below(6) {
        0 => format!("CREATE INDEX IF NOT EXISTS ix{k} ON t{t}(a)"),
        1 => format!("DROP INDEX IF EXISTS ix{k}"),
        2 => format!("ALTER TABLE t{t} ADD COLUMN e{} DEFAULT {}", r.below(1000), lit(r)),
        3 => format!("CREATE TABLE IF NOT EXISTS tmp{k}(x)"),
        4 => format!("DROP TABLE IF EXISTS tmp{k}"),
        _ => format!("INSERT INTO tmp{k} VALUES ({})", lit(r)),
    }
}

/// The round-1 generator, unchanged (kept for the coverage A/B).
pub fn session_v1(seed: u64) -> Session {
    let mut r = Rng::new(seed);
    let mut setup_sql = Vec::new();
    let schema = setup(&mut r, &mut setup_sql);
    let conns = if r.chance(0.3) { 2 } else { 1 };
    let nslots = 2 + r.below(4);
    let mut slots = vec![(Slot::Empty, 0usize, 0usize, false); nslots]; // state, conn, params, writer
    let mut savepoints = 0usize;
    let mut actions = Vec::new();
    let len = 6 + r.below(36);
    while actions.len() < len {
        let running = slots.iter().any(|s| s.0 == Slot::Running);
        let running_writer = slots.iter().any(|s| s.0 == Slot::Running && s.3);
        let empty: Vec<usize> = (0..nslots).filter(|&k| slots[k].0 == Slot::Empty).collect();
        let live: Vec<usize> = (0..nslots).filter(|&k| slots[k].0 != Slot::Empty).collect();
        let ready_params: Vec<usize> = (0..nslots).filter(|&k| slots[k].0 == Slot::Ready && slots[k].2 > 0).collect();
        let conn = if conns == 2 && r.chance(0.3) { 1 } else { 0 };
        let w = [
            if empty.is_empty() { 0 } else { 4 },                         // prepare
            if live.is_empty() { 0 } else { 6 },                          // step
            if running { 6 } else { 3 },                                  // exec txn
            if running && !running_writer { 4 } else { 2 },               // exec write / ddl
            if live.is_empty() { 0 } else { 1 },                          // reset
            if live.is_empty() { 0 } else { 1 },                          // finalize
            if ready_params.is_empty() { 0 } else { 3 },                  // bind
            if running_writer { 3 } else { 0 },                           // finish the writer
        ];
        match r.weighted(&w) {
            0 => {
                let slot = *r.pick(&empty);
                let (sql, params, writer) = match r.weighted(&[10, 7, 2, 1]) {
                    0 => {
                        let (s, p) = read(&mut r, &schema);
                        (s, p, false)
                    },
                    1 => {
                        let (s, p) = write(&mut r, &schema);
                        (s, p, true)
                    },
                    2 => (txn(&mut r, &mut savepoints), 0, false),
                    _ => (ddl(&mut r, &schema), 0, true),
                };
                actions.push(Action::Prepare { slot, conn, sql });
                slots[slot] = (Slot::Ready, conn, params, writer);
                if params > 0 && r.chance(0.8) {
                    for idx in 1..=params {
                        let val = if r.chance(0.5) { small_int(&mut r) } else { value(&mut r) };
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
            2 => actions.push(Action::Exec { conn, sql: txn(&mut r, &mut savepoints) }),
            3 => {
                let sql = if r.chance(0.8) {
                    let (s, p) = write(&mut r, &schema);
                    if p > 0 {
                        // One-shot statements get their parameters inline.
                        let mut s = s;
                        for idx in (1..=p).rev() {
                            s = s.replace(&format!("?{idx}"), &small_int(&mut r).sql());
                        }
                        s
                    } else {
                        s
                    }
                } else {
                    ddl(&mut r, &schema)
                };
                actions.push(Action::Exec { conn, sql });
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
                let val = value(&mut r);
                actions.push(Action::Bind { slot, idx, val });
            },
            _ => {
                let ws: Vec<usize> = (0..nslots).filter(|&k| slots[k].0 == Slot::Running && slots[k].3).collect();
                let slot = *r.pick(&ws);
                actions.push(Action::Step { slot, n: STEP_ALL });
                slots[slot].0 = Slot::Halted;
            },
        }
    }
    let _ = (schema.fk, schema.triggers);
    Session { seed, conns, mvcc: false, setup: setup_sql, actions }
}
