// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Sessions: the action language every generator shares (README.md,
//! "Generators"): a schema and data (setup, run on connection 0), then API
//! actions over a few statement slots on one or two connections; the value
//! pools; the choice of generator version (`--gen 1|2|3`, default 3); and the
//! session scripts (`render` / `parse`) used by findings, `replay` and the
//! bug folders' found-by.txt.
//!
//! - [`v1`]: round 1, and the building blocks (schema, reads, writes,
//!   transaction control, DDL) that v2 and v3 reuse;
//! - [`v2`]: round 2: virtual tables, ATTACH, re-prepare weighting;
//! - [`v3`]: round 3: ATTACH depth, MVCC, vtab RENAME/DROP.
//!
//! The older versions stay selectable as the baselines for coverage A/B runs.

pub mod v1;
pub mod v2;
pub mod v3;

use crate::engines::Cell;

pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed ^ 0x9E37_79B9_7F4A_7C15)
    }
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    pub fn chance(&mut self, p: f64) -> bool {
        ((self.next() >> 11) as f64 / (1u64 << 53) as f64) < p
    }
    pub fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len())]
    }
    /// An index by weight.
    pub fn weighted(&mut self, ws: &[u32]) -> usize {
        let total: u32 = ws.iter().sum();
        let mut x = (self.next() % total.max(1) as u64) as u32;
        for (k, w) in ws.iter().enumerate() {
            if x < *w {
                return k;
            }
            x -= w;
        }
        ws.len() - 1
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Prepare { slot: usize, conn: usize, sql: String },
    Bind { slot: usize, idx: usize, val: Cell },
    /// Step up to `n` times (stops at done or an error).
    Step { slot: usize, n: usize },
    Reset { slot: usize },
    Finalize { slot: usize },
    /// Prepare, step to the end, finalize: one statement on its own.
    Exec { conn: usize, sql: String },
}

impl Action {
    pub fn kind(&self) -> &'static str {
        match self {
            Action::Prepare { .. } => "prepare",
            Action::Bind { .. } => "bind",
            Action::Step { .. } => "step",
            Action::Reset { .. } => "reset",
            Action::Finalize { .. } => "finalize",
            Action::Exec { .. } => "exec",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Session {
    pub seed: u64,
    pub conns: usize,
    /// Round 3: Turso opens the database in MVCC mode (SQLite stays WAL).
    pub mvcc: bool,
    pub setup: Vec<String>,
    pub actions: Vec<Action>,
}

pub const STEP_ALL: usize = 500;

const TEXTS: &[&str] = &["a", "b", "ab", "", "10", "1e3", " 5", "x y", "B", "zz", "-0", "3.0", "Ab", "\u{e9}t\u{e9}"];
const REALS: &[f64] = &[0.5, 1.0, -2.5, 1e20, 3.0, -0.0, 0.1, 2.0, 1e-7, 123456789.125];
const BLOBS: &[&[u8]] = &[b"", b"\x00", b"AB", b"\x01\x02\x03", b"10"];

pub fn value(r: &mut Rng) -> Cell {
    match r.weighted(&[2, 6, 3, 4, 2]) {
        0 => Cell::Null,
        1 => Cell::Int(*r.pick(&[-3i64, -1, 0, 1, 2, 3, 5, 7, 10, 13, 20, 99, 1 << 40, i64::MAX, i64::MIN])),
        2 => Cell::Real(*r.pick(REALS)),
        3 => Cell::Text(r.pick(TEXTS).to_string()),
        _ => Cell::Blob(r.pick(BLOBS).to_vec()),
    }
}

pub(crate) fn small_int(r: &mut Rng) -> Cell {
    Cell::Int(r.below(12) as i64 - 1)
}

pub(crate) fn lit(r: &mut Rng) -> String {
    value(r).sql()
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Slot {
    Empty,
    /// Fresh or reset: bindable.
    Ready,
    Running,
    /// Stepped to the end (as far as the generator knows).
    Halted,
}

pub use v1::session_v1;
pub use v2::session_v2;
pub use v3::session_v3;

/// The generator `run` and `one` use (round 3 by default; `--gen 1` or `--gen 2` for earlier rounds).
pub fn session(seed: u64, version: u32) -> Session {
    match version {
        1 => session_v1(seed),
        2 => session_v2(seed),
        _ => session_v3(seed),
    }
}

// ------------------------------------------------------------ scripts

/// A session as a readable script, parsed back by [`parse`].
pub fn render(s: &Session) -> String {
    let mut out = format!("-- turso-apidiff session, seed {}\n-- conns {}\n", s.seed, s.conns);
    if s.mvcc {
        out.push_str("-- mvcc 1\n");
    }
    out.push_str("-- setup (connection 0)\n");
    for q in &s.setup {
        out.push_str(&format!("{q};\n"));
    }
    out.push_str("-- actions\n");
    for a in &s.actions {
        out.push_str(&render_action(a));
        out.push('\n');
    }
    out
}

pub fn render_action(a: &Action) -> String {
    match a {
        Action::Prepare { slot, conn, sql } => format!("prepare s{slot} c{conn}: {sql}"),
        Action::Bind { slot, idx, val } => format!("bind s{slot} ?{idx} = {}", val.sql()),
        Action::Step { slot, n } => {
            if *n >= STEP_ALL {
                format!("step s{slot} all")
            } else {
                format!("step s{slot} x{n}")
            }
        },
        Action::Reset { slot } => format!("reset s{slot}"),
        Action::Finalize { slot } => format!("finalize s{slot}"),
        Action::Exec { conn, sql } => format!("exec c{conn}: {sql}"),
    }
}

fn num(s: &str, prefix: char) -> Result<usize, String> {
    s.strip_prefix(prefix).and_then(|x| x.parse().ok()).ok_or_else(|| format!("expected {prefix}N, got {s:?}"))
}

/// A literal (as [`Cell::sql`] writes it) back to a value, through SQLite.
fn parse_literal(s: &str) -> Result<Cell, String> {
    let db = rusqlite::Connection::open_in_memory().map_err(|e| e.to_string())?;
    db.query_row(&format!("SELECT {s}"), [], |row| {
        Ok(match row.get_ref(0)? {
            rusqlite::types::ValueRef::Null => Cell::Null,
            rusqlite::types::ValueRef::Integer(i) => Cell::Int(i),
            rusqlite::types::ValueRef::Real(x) => Cell::Real(x),
            rusqlite::types::ValueRef::Text(t) => Cell::Text(String::from_utf8_lossy(t).into_owned()),
            rusqlite::types::ValueRef::Blob(b) => Cell::Blob(b.to_vec()),
        })
    })
    .map_err(|e| format!("literal {s:?}: {e}"))
}

pub fn parse(text: &str) -> Result<Session, String> {
    let mut s = Session { seed: 0, conns: 1, mvcc: false, setup: Vec::new(), actions: Vec::new() };
    let mut in_actions = false;
    for line in text.lines() {
        let l = line.trim_end();
        if let Some(rest) = l.strip_prefix("-- turso-apidiff session, seed ") {
            s.seed = rest.trim().parse().unwrap_or(0);
            continue;
        }
        if let Some(rest) = l.strip_prefix("-- conns ") {
            s.conns = rest.trim().parse().map_err(|_| "bad conns".to_string())?;
            continue;
        }
        if l == "-- mvcc 1" {
            s.mvcc = true;
            continue;
        }
        if l == "-- actions" {
            in_actions = true;
            continue;
        }
        if l.is_empty() || l.starts_with("--") {
            continue;
        }
        if !in_actions {
            s.setup.push(l.trim_end_matches(';').to_string());
            continue;
        }
        let (head, sql) = match l.split_once(": ") {
            Some((h, q)) if h.starts_with("prepare") || h.starts_with("exec") => (h, Some(q.to_string())),
            _ => (l, None),
        };
        let w: Vec<&str> = head.split_whitespace().collect();
        let a = match w[0] {
            "prepare" => Action::Prepare { slot: num(w[1], 's')?, conn: num(w[2], 'c')?, sql: sql.unwrap() },
            "exec" => Action::Exec { conn: num(w[1], 'c')?, sql: sql.unwrap() },
            "bind" => {
                let (_, v) = l.split_once(" = ").ok_or("bind without =")?;
                Action::Bind { slot: num(w[1], 's')?, idx: num(w[2], '?')?, val: parse_literal(v)? }
            },
            "step" => Action::Step { slot: num(w[1], 's')?, n: if w[2] == "all" { STEP_ALL } else { num(w[2], 'x')? } },
            "reset" => Action::Reset { slot: num(w[1], 's')? },
            "finalize" => Action::Finalize { slot: num(w[1], 's')? },
            other => return Err(format!("unknown action {other:?}")),
        };
        s.actions.push(a);
    }
    Ok(s)
}
