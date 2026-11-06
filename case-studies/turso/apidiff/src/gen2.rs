// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Round-2 generator (README.md, "Generators"). Same action language
//! and script format as round 1 ([`crate::gen`]); on top of round 1:
//! - the writable application virtual table `appvt` ([`crate::vt`]):
//!   vt0/vt1 in 45% of sessions, single-row INSERT/UPDATE/DELETE that can
//!   fail before or after writing the module's storage (a = 13, a = 99,
//!   b = 'boom', an existing rowid), with conflict clauses, inside
//!   BEGIN/SAVEPOINT/ROLLBACK TO, interleaved with suspended statements;
//!   reads, joins and windows over it; vtab CREATE/DROP as DDL;
//! - ATTACH (25%): `aux.u0` on the engine's own second file, attached on
//!   both connections (two connections in 60% of those), so BUSY can come
//!   from the attached database;
//! - re-weighting toward round 1's productive areas: DDL (also from the
//!   other connection) while statements are prepared but not stepped
//!   (re-prepare), window/aggregate reads, reset/finalize while another
//!   statement is suspended;
//! - fewer transactions opened by SAVEPOINT on two-connection sessions
//!   (known #11394): BEGIN first, 80% of the time, when the generator
//!   believes the connection is in autocommit.

use crate::{
    engines::Cell,
    gen::{conflict, ddl, lit, read, setup, small_int, txn, write, Action, Rng, Schema, Session, Slot, STEP_ALL},
};

pub(crate) struct S2 {
    pub(crate) base: Schema,
    /// Ordinary tables: t0.. and aux.u0 when attached.
    pub(crate) names: Vec<String>,
    pub(crate) vtabs: Vec<String>,
    pub(crate) attach: bool,
}

/// `t3` -> another table name, whole words only.
pub(crate) fn retarget(sql: &str, from: &str, to: &str) -> String {
    let b = sql.as_bytes();
    let f = from.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len() + 8);
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut i = 0;
    while i < b.len() {
        let word_start = i == 0 || !(word(b[i - 1]) || b[i - 1] == b'.');
        if word_start && b[i..].starts_with(f) {
            let j = i + f.len();
            if j == b.len() || !word(b[j]) {
                out.extend_from_slice(to.as_bytes());
                i = j;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8(out).expect("ASCII-only replacement keeps UTF-8")
}

pub(crate) fn vt_a(r: &mut Rng) -> String {
    match r.weighted(&[6, 2, 1, 2]) {
        0 => small_int(r).sql(),
        1 => "13".into(),
        2 => "99".into(),
        _ => lit(r),
    }
}

pub(crate) fn vt_b(r: &mut Rng) -> String {
    if r.chance(0.1) {
        "'boom'".into()
    } else {
        lit(r)
    }
}

pub(crate) fn setup2(r: &mut Rng, out: &mut Vec<String>) -> S2 {
    let base = setup(r, out);
    // Coverage A/B (README.md): round-2 sessions held too few rows to split
    // pages (balance_non_root, interior index moves). A deterministic bulk
    // insert with wide text in a third of the sessions.
    if r.chance(0.35) {
        let t = r.below(base.tables);
        let n = *r.pick(&[60, 150, 300]);
        let w = *r.pick(&[20, 200, 700]);
        let pcol = if base.fk && t > 0 { ", p" } else { "" };
        let pval = if base.fk && t > 0 { ", NULL" } else { "" };
        out.push(format!(
            "INSERT OR IGNORE INTO t{t}(a, b, c, d{pcol}) WITH RECURSIVE s(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM s WHERE n < {n}) SELECT n % 17, printf('%0{w}d', n), n * 0.5, zeroblob(n % 50){pval} FROM s"
        ));
    }
    let mut names: Vec<String> = (0..base.tables).map(|t| format!("t{t}")).collect();
    let mut vtabs = Vec::new();
    if r.chance(0.45) {
        let n = if r.chance(0.3) { 2 } else { 1 };
        for v in 0..n {
            let name = format!("vt{v}");
            out.push(format!("CREATE VIRTUAL TABLE {name} USING appvt({name})"));
            for _ in 0..r.below(6) {
                out.push(format!("INSERT INTO {name}(a, b) VALUES ({}, {})", small_int(r).sql(), lit(r)));
            }
            vtabs.push(name);
        }
    }
    let attach = r.chance(0.25);
    if attach {
        out.push(format!("ATTACH '{}' AS aux", crate::engines::AUX));
        out.push("CREATE TABLE aux.u0(id INTEGER PRIMARY KEY, a INT, b TEXT, c REAL, d)".into());
        if r.chance(0.5) {
            out.push("CREATE INDEX aux.u0_b ON u0(b)".into());
        }
        let n = r.below(8);
        if n > 0 {
            let rows: Vec<String> = (0..n).map(|_| format!("({}, {}, {}, {})", small_int(r).sql(), lit(r), lit(r), lit(r))).collect();
            out.push(format!("INSERT INTO aux.u0(a, b, c, d) VALUES {}", rows.join(", ")));
        }
        names.push("aux.u0".into());
    }
    S2 { base, names, vtabs, attach }
}

pub(crate) fn table(r: &mut Rng, s: &S2) -> String {
    r.pick(&s.names).clone()
}

/// Round-1 templates, sometimes moved to aux.u0.
pub(crate) fn moved(r: &mut Rng, s: &S2, sql: String) -> String {
    if s.attach && r.chance(0.35) {
        let from = format!("t{}", r.below(s.base.tables));
        if sql.contains(&from) {
            return retarget(&sql, &from, "aux.u0");
        }
    }
    sql
}

pub(crate) fn read2(r: &mut Rng, s: &S2) -> (String, usize) {
    let t = table(r, s);
    let vt = if s.vtabs.is_empty() { None } else { Some(r.pick(&s.vtabs).clone()) };
    let k = r.weighted(&[8, 6, 4]);
    if k == 2 {
        if let Some(v) = vt {
            return match r.below(7) {
                0 => (format!("SELECT rowid, a, b FROM {v} ORDER BY rowid"), 0),
                1 => (format!("SELECT count(*), sum(a), max(rowid), app_isum(a) FROM {v}"), 0),
                2 => (format!("SELECT v.rowid, v.a, x.id FROM {v} v JOIN {t} x ON x.a = v.a ORDER BY 1, 3"), 0),
                3 => (format!("SELECT rowid, a, b FROM {v} WHERE a > ?1 ORDER BY rowid"), 1),
                4 => (format!("SELECT rowid, app_text(a), app_type(b) FROM {v} WHERE rowid = ?1"), 1),
                5 => (format!("SELECT rowid, a, sum(rowid) OVER (ORDER BY rowid), count(b) OVER (ORDER BY rowid ROWS BETWEEN 1 PRECEDING AND CURRENT ROW) FROM {v} ORDER BY rowid"), 0),
                _ => (format!("SELECT a, count(*), sum(rowid) FROM {v} GROUP BY a ORDER BY a"), 0),
            };
        }
    }
    if k == 1 {
        // Windows and aggregates (the panic and E came from there).
        return match r.below(9) {
            0 => (format!("SELECT id, row_number() OVER (ORDER BY id), sum(a) OVER (PARTITION BY a % 2 ORDER BY id) FROM {t} ORDER BY id"), 0),
            1 => (format!("SELECT id, lag(a) OVER (ORDER BY id), lead(b) OVER (ORDER BY id) FROM {t} ORDER BY id"), 0),
            2 => (format!("SELECT id, count(*) OVER (ORDER BY id ROWS BETWEEN 1 PRECEDING AND CURRENT ROW), max(id) OVER () FROM {t} ORDER BY id"), 0),
            3 => (format!("SELECT a, rank() OVER (ORDER BY a), dense_rank() OVER (ORDER BY a) FROM {t} ORDER BY a, id"), 0),
            4 => (format!("SELECT a, count(*), sum(id), min(b), max(c), total(c) FROM {t} GROUP BY a ORDER BY a"), 0),
            5 => (format!("SELECT count(*), sum(a), max(c) FROM {t} WHERE id > ?1"), 1),
            6 => (format!("SELECT a, count(*) FROM {t} GROUP BY a HAVING count(*) > ?1 ORDER BY a"), 1),
            7 => (format!("SELECT id, sum(id) OVER (ORDER BY id RANGE BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW), app_echo(a) FROM {t} ORDER BY id"), 0),
            _ => (format!("SELECT x.id, count(y.id) OVER (ORDER BY x.id) FROM {t} x LEFT JOIN {} y ON y.a = x.id ORDER BY x.id", table(r, s)), 0),
        };
    }
    let (q, p) = read(r, &s.base);
    (moved(r, s, q), p)
}

/// A write; `vt_ok`: virtual-table writes allowed (connection 0 only:
/// the module's storage is shared by the connections).
pub(crate) fn write2(r: &mut Rng, s: &S2, vt_ok: bool) -> (String, usize) {
    if vt_ok && !s.vtabs.is_empty() && r.chance(0.4) {
        let v = r.pick(&s.vtabs).clone();
        let n = 1 + r.below(8);
        return match r.below(13) {
            0 | 1 => (format!("INSERT{} INTO {v}(a, b) VALUES (?1, ?2)", conflict(r)), 2),
            2 | 3 => (format!("INSERT{} INTO {v}(a, b) VALUES ({}, {})", conflict(r), vt_a(r), vt_b(r)), 0),
            // Multi-row: rare (Turso inserts only the last row; candidate).
            4 if r.chance(0.15) => (format!("INSERT INTO {v}(a, b) VALUES ({}, {}), ({}, {})", vt_a(r), vt_b(r), vt_a(r), vt_b(r)), 0),
            4 => (format!("INSERT INTO {v}(a, b) VALUES ({}, {})", vt_a(r), vt_b(r)), 0),
            5 => (format!("INSERT INTO {v}(rowid, a, b) VALUES ({n}, {}, {})", vt_a(r), vt_b(r)), 0),
            6 => (format!("UPDATE{} {v} SET a = a + 1 WHERE rowid = {n}", conflict(r)), 0),
            7 => (format!("UPDATE{} {v} SET a = {}, b = ?1 WHERE a > ?2", conflict(r), r.pick(&["13", "99", "a * 2", "a + 1", "NULL"])), 2),
            8 => (format!("UPDATE {v} SET b = {} WHERE rowid = {n}", vt_b(r)), 0),
            // Known panic (candidate turso-vtab-update-rowid-panics): rare.
            9 if r.chance(0.1) => (format!("UPDATE {v} SET rowid = rowid + {} WHERE rowid = {n}", r.pick(&[1, 10])), 0),
            9 => (format!("UPDATE {v} SET a = a - 1 WHERE rowid = {n}"), 0),
            10 => (format!("DELETE FROM {v} WHERE rowid = {n}"), 0),
            11 => (format!("DELETE FROM {v} WHERE a {}", r.pick(&["= 99", "> 3", "IS NULL"])), 0),
            _ => {
                let t = table(r, s);
                if r.chance(0.5) {
                    (format!("INSERT INTO {t}(a, b) SELECT a, b FROM {v} ORDER BY rowid"), 0)
                } else {
                    (format!("UPDATE {t} SET c = (SELECT count(*) FROM {v}) WHERE id = ?1"), 1)
                }
            },
        };
    }
    let (q, p) = write(r, &s.base);
    (moved(r, s, q), p)
}

pub(crate) fn ddl2(r: &mut Rng, s: &S2) -> String {
    let t = table(r, s);
    let k = r.below(3);
    match r.below(10) {
        0 if s.attach => format!("CREATE INDEX IF NOT EXISTS aux.ux{k} ON u0(a, b)"),
        1 if s.attach => format!("ALTER TABLE aux.u0 ADD COLUMN e{} DEFAULT {}", r.below(1000), lit(r)),
        // Not IF NOT EXISTS: Turso's parser rejects it (candidate).
        2 if !s.vtabs.is_empty() => "CREATE VIRTUAL TABLE vt9 USING appvt(vt9)".into(),
        3 if !s.vtabs.is_empty() => "DROP TABLE IF EXISTS vt9".into(),
        4 => format!("CREATE INDEX IF NOT EXISTS ix{k} ON {}(b, a)", t.trim_start_matches("aux.")).replace("ON u0", "ON t0"),
        5 => format!("DROP INDEX IF EXISTS ix{k}"),
        6 => format!("CREATE TABLE IF NOT EXISTS tmp{k}(x)"),
        _ => ddl(r, &s.base),
    }
}

pub fn session_v2(seed: u64) -> Session {
    let mut r = Rng::new(seed);
    let mut setup_sql = Vec::new();
    let schema = setup2(&mut r, &mut setup_sql);
    let conns = if r.chance(if schema.attach { 0.6 } else { 0.3 }) { 2 } else { 1 };
    let nslots = 2 + r.below(4);
    let mut slots = vec![(Slot::Empty, 0usize, 0usize, false); nslots]; // state, conn, params, writer
    let mut savepoints = 0usize;
    let mut actions = Vec::new();
    // What the generator believes: connection k is inside a transaction.
    let mut in_txn = [false; 2];
    // ... and has read or written in it (SQLite's snapshot exists).
    let mut touched = [false; 2];
    if schema.attach && conns == 2 {
        actions.push(Action::Exec { conn: 1, sql: format!("ATTACH '{}' AS aux", crate::engines::AUX) });
    }
    let len = 6 + r.below(36);
    let note_txn = |sql: &str, conn: usize, in_txn: &mut [bool; 2]| {
        let u = sql.to_uppercase();
        if u.starts_with("BEGIN") || u.starts_with("SAVEPOINT") {
            in_txn[conn] = true;
        } else if u.starts_with("COMMIT") || u.starts_with("END") || (u.starts_with("ROLLBACK") && !u.starts_with("ROLLBACK TO")) {
            in_txn[conn] = false;
        }
    };
    while actions.len() < len {
        let running = slots.iter().filter(|s| s.0 == Slot::Running).count();
        let running_writer = slots.iter().any(|s| s.0 == Slot::Running && s.3);
        let ready = slots.iter().any(|s| s.0 == Slot::Ready);
        let empty: Vec<usize> = (0..nslots).filter(|&k| slots[k].0 == Slot::Empty).collect();
        let live: Vec<usize> = (0..nslots).filter(|&k| slots[k].0 != Slot::Empty).collect();
        let ready_params: Vec<usize> = (0..nslots).filter(|&k| slots[k].0 == Slot::Ready && slots[k].2 > 0).collect();
        let conn = if conns == 2 && r.chance(0.3) { 1 } else { 0 };
        let w = [
            if empty.is_empty() { 0 } else { 4 },                                  // prepare
            if live.is_empty() { 0 } else { 6 },                                   // step
            if running > 0 { 6 } else { 3 },                                       // exec txn
            if running > 0 && !running_writer { 4 } else { 2 },                    // exec write
            if live.is_empty() { 0 } else if running >= 2 { 3 } else { 1 },        // reset
            if live.is_empty() { 0 } else if running >= 2 { 2 } else { 1 },        // finalize
            if ready_params.is_empty() { 0 } else { 3 },                           // bind
            if running_writer { 3 } else { 0 },                                    // finish the writer
            if ready || running > 0 { 3 } else { 1 },                              // exec DDL (re-prepare)
        ];
        match r.weighted(&w) {
            0 => {
                let slot = *r.pick(&empty);
                let (sql, params, writer) = match r.weighted(&[10, 7, 2, 1]) {
                    0 => {
                        let (s, p) = read2(&mut r, &schema);
                        (s, p, false)
                    },
                    1 => {
                        let (s, p) = write2(&mut r, &schema, conn == 0);
                        (s, p, true)
                    },
                    2 => (txn(&mut r, &mut savepoints), 0, false),
                    _ => (ddl2(&mut r, &schema), 0, true),
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
                let sql = txn(&mut r, &mut savepoints);
                // Known #11394/#10825 on two connections: Turso takes the
                // snapshot at SAVEPOINT (also after a deferred BEGIN) and
                // at a statement that reads only the attached database;
                // read main first most of the time so both engines hold it.
                if conns == 2 && (sql.starts_with("SAVEPOINT") || sql == "BEGIN") && !touched[conn] && r.chance(0.8) {
                    if sql.starts_with("SAVEPOINT") && !in_txn[conn] {
                        actions.push(Action::Exec { conn, sql: "BEGIN".into() });
                        in_txn[conn] = true;
                    }
                    if sql == "BEGIN" && !in_txn[conn] {
                        actions.push(Action::Exec { conn, sql: "BEGIN".into() });
                        in_txn[conn] = true;
                    }
                    let touch = if schema.attach { "SELECT (SELECT count(*) FROM t0), (SELECT count(*) FROM aux.u0)" } else { "SELECT count(*) FROM t0" };
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
                let (s, p) = write2(&mut r, &schema, conn == 0);
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
            _ => {
                // DDL, from the other connection half the time when there
                // are two (schema changes under prepared statements).
                let c = if conns == 2 && r.chance(0.5) { 1 } else { 0 };
                actions.push(Action::Exec { conn: c, sql: ddl2(&mut r, &schema) });
            },
        }
    }
    let _ = (schema.base.fk, schema.base.triggers);
    Session { seed, conns, mvcc: false, setup: setup_sql, actions }
}
