// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! turso-apidiff: an API-interleaving differential of Turso (turso_core,
//! latest main) against SQLite 3.50.4.
//!
//! A session ([`gen::session`]) is a schema and data, then API actions over
//! a few prepared-statement slots on one connection (sometimes two on the
//! same file): prepare, bind, step k times, reset, finalize, and one-shot
//! statements (BEGIN/COMMIT/ROLLBACK/SAVEPOINT/RELEASE/ROLLBACK TO, writes,
//! DDL) run while other statements are suspended partway. Both engines run
//! the session in lockstep and every action's observable result is compared:
//! the result class of each prepare/step/bind/reset (messages are not
//! compared), the rows, and per connection changes(), total_changes(),
//! last_insert_rowid() and autocommit. At the end, all tables are dumped and
//! integrity_check runs in the live session, open transactions are
//! committed, and both files are reopened with SQLite (Turso's file read by
//! SQLite) and compared again.
//!
//! The first divergence ends a session. It is classified ([`classify`]):
//! a Turso panic or an unexplained difference is a finding; a feature Turso
//! rejects as unsupported, or one of COMPAT.md's documented
//! same-connection-writer gaps, is a skip (counted by reason); a difference
//! in rows of a statement that was suspended while the same connection
//! wrote to a table it reads is "unspecified" (SQLite leaves it undefined
//! whether such a statement sees the change) and kept apart.
//!
//! Usage (from this directory):
//!   cargo run --release -- run --jobs J --minutes M [--seed S] [--out DIR]
//!   cargo run --release -- one SEED          # trace one session
//!   cargo run --release -- replay FILE       # trace a session script
//!   cargo run --release -- shrink FILE       # shrink a session script

mod engines;
mod gen;
mod gen2;
mod gen3;
mod vt;

use std::{
    collections::BTreeMap,
    fmt::Write as _,
    fs,
    panic::{self, AssertUnwindSafe},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use engines::{Cell, Class, ConnState, Engine, Ev, Sqlite, Turso};
use std::sync::atomic::AtomicU32;

/// The generator version `run`/`one` use (`--gen`, default 3).
static GEN: AtomicU32 = AtomicU32::new(3);
use gen::{Action, Session, STEP_ALL};

const EXAMPLES: usize = 3;
const HANG: Duration = Duration::from_secs(60);
const HIDDEN: usize = 1000;

thread_local! {
    static PANIC_AT: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

fn panic_message(p: &(dyn std::any::Any + Send)) -> String {
    let m = if let Some(s) = p.downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = p.downcast_ref::<String>() {
        s.clone()
    } else {
        "(non-string panic)".into()
    };
    let at = PANIC_AT.with(|a| a.borrow().clone());
    format!("{m} at {at}")
}

/// Runs `f` on Turso, a panic becoming [`Ev::Panic`].
fn guarded<T>(f: impl FnOnce() -> T, wrap: impl FnOnce(String) -> T) -> T {
    match panic::catch_unwind(AssertUnwindSafe(f)) {
        Ok(v) => v,
        Err(p) => wrap(panic_message(&*p)),
    }
}

// ------------------------------------------------------------ helpers

fn normalize(e: &str) -> String {
    let mut out = String::new();
    let mut chars = e.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() {
            while chars.peek().is_some_and(|c| c.is_ascii_digit()) {
                chars.next();
            }
            out.push('N');
        } else if c == '\'' || c == '"' || c == '`' {
            for d in chars.by_ref() {
                if d == c {
                    break;
                }
            }
            out.push_str("'?'");
        } else {
            out.push(c);
        }
    }
    let out: String = out.split_whitespace().collect::<Vec<_>>().join(" ");
    out.chars().take(100).collect()
}

/// A Turso error naming a missing feature (turso-diff's rule).
fn unsupported(e: &str) -> bool {
    let l = e.to_lowercase();
    ["not supported", "unsupported", "not yet supported", "not implemented", "not yet implemented", "is disabled", "experimental", "only support"]
        .iter()
        .any(|p| l.contains(p))
}

fn cells_equal(a: &[Cell], b: &[Cell]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(p, q)| match (p, q) {
            (Cell::Real(u), Cell::Real(v)) => u == v || (u - v).abs() <= 1e-12 * u.abs().max(v.abs()),
            _ => p == q,
        })
}

/// Whole-number REALs as integers: SQLite 3.50.4 returns an INTEGER for a
/// REAL column's value passed through a function in RETURNING
/// (MEM_IntReal), where Turso returns the REAL (see README.md).
fn canonical(r: &[Cell]) -> Vec<Cell> {
    r.iter()
        .map(|c| match c {
            Cell::Real(x) if x.fract() == 0.0 && x.abs() < 9.007_199_254_740_992e15 && !(*x == 0.0 && x.is_sign_negative()) => Cell::Int(*x as i64),
            c => c.clone(),
        })
        .collect()
}

fn show_cells(r: &[Cell]) -> String {
    r.iter().map(Cell::sql).collect::<Vec<_>>().join("|")
}

fn show_ev(e: &Ev) -> String {
    match e {
        Ev::Ok => "ok".into(),
        Ev::Row(r) => format!("row {}", show_cells(r)),
        Ev::Done => "done".into(),
        Ev::Err(c, m) => format!("{c:?}: {m}"),
        Ev::NoSlot => "no-slot".into(),
        Ev::Panic(m) => format!("PANIC {m}"),
    }
}

fn ev_kind(e: &Ev) -> String {
    match e {
        Ev::Ok => "ok".into(),
        Ev::Row(_) => "row".into(),
        Ev::Done => "done".into(),
        Ev::Err(c, _) => format!("{c:?}"),
        Ev::NoSlot => "no-slot".into(),
        Ev::Panic(_) => "panic".into(),
    }
}

fn show_state(s: &ConnState) -> String {
    format!("auto={} changes={} total={} rowid={}", u8::from(s.autocommit), s.changes, s.total_changes, s.last_rowid)
}

/// The statement kind for signatures: its first keyword(s).
fn stmt_kind(sql: &str) -> String {
    let w: Vec<String> = sql.split_whitespace().take(3).map(|s| s.to_uppercase()).collect();
    let mut k = match w.first().map(String::as_str) {
        Some("ROLLBACK") if w.get(1).is_some_and(|x| x == "TO" || x == "TRANSACTION") => "ROLLBACK TO".to_string(),
        Some("CREATE") if w.get(1).is_some_and(|x| x == "INDEX" || x == "UNIQUE") => "CREATE INDEX".into(),
        Some("CREATE") | Some("DROP") => format!("{} {}", w[0], w.get(1).cloned().unwrap_or_default()),
        Some("BEGIN") => "BEGIN".into(),
        Some(x) => x.to_string(),
        None => String::new(),
    };
    if sql.to_uppercase().contains(" RETURNING ") {
        k.push_str(" RETURNING");
    }
    k
}

/// Tables a statement names (the generator's t0.., tmp0..).
fn tables_of(sql: &str) -> Vec<String> {
    let mut out = Vec::new();
    for tok in sql.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
        let num = |p: &str| tok.len() > p.len() && tok.starts_with(p) && tok[p.len()..].chars().all(|c| c.is_ascii_digit());
        let is_t = num("t") || num("tmp") || num("u") || num("vt") || num("w");
        if is_t && !out.contains(&tok.to_string()) {
            out.push(tok.to_string());
        }
    }
    out
}

// ------------------------------------------------------------ session run

#[derive(Default, Clone)]
struct SlotMeta {
    sql: String,
    conn: usize,
    writer: bool,
    returning: bool,
    /// Last step gave a row: suspended partway.
    suspended: bool,
    /// Last step gave done or a (non-BUSY) error.
    halted: bool,
    stepped: bool,
    overlapped: bool,
    rows_s: Vec<Vec<Cell>>,
    rows_t: Vec<Vec<Cell>>,
}

#[derive(Clone, Debug, PartialEq)]
enum Verdict {
    Clean,
    Finding,
    Unspecified,
    Skip,
    /// A divergence after a pattern of an already-triaged bug (README.md).
    Known,
}

#[derive(Clone, Debug)]
struct Divergence {
    verdict: Verdict,
    signature: String,
    detail: String,
}

#[derive(Default)]
struct Counters(BTreeMap<String, u64>);

impl Counters {
    fn add(&mut self, k: impl Into<String>, n: u64) {
        *self.0.entry(k.into()).or_default() += n;
    }
    fn merge(&mut self, o: &Counters) {
        for (k, v) in &o.0 {
            self.add(k.clone(), *v);
        }
    }
}

struct Runner<'a> {
    s: Sqlite,
    t: Option<Turso>,
    meta: BTreeMap<usize, SlotMeta>,
    /// COMPAT.md: a write statement inside BEGIN reset or dropped before it
    /// finished makes the transaction rollback-only.
    tainted: Option<String>,
    log: Vec<String>,
    stats: &'a mut Counters,
    verbose: bool,
    /// The schema has triggers or foreign keys: a write can touch any table.
    cascades: bool,
    /// The slot of the last Step action.
    last_slot: Option<usize>,
    /// Per connection: a CREATE INDEX finished since the last DML did
    /// (Turso sets changes() to 0 there, SQLite keeps it; candidate
    /// turso-create-index-resets-changes).
    index_changes: Vec<bool>,
    /// Per connection: a SAVEPOINT started the transaction (turso #11394:
    /// Turso takes the snapshot at once, SQLite at the first read).
    sp_begin: Vec<bool>,
    /// Per connection: a window-function query ran since the last INSERT
    /// (turso #8698: it overwrites last_insert_rowid()).
    window_rowid: Vec<bool>,
    /// Patterns of the triaged candidates seen so far (README.md, "Classification").
    known: Vec<&'static str>,
    /// Count of finished DDL statements, and per slot the count at prepare.
    schema_epoch: u64,
    epochs: BTreeMap<usize, u64>,
    /// Round 2: aux is attached on connection 0; virtual tables of the setup.
    attach: bool,
    vtabs: Vec<String>,
    /// A ROLLBACK TO ran (candidate G's pattern).
    saw_rollback_to: bool,
    /// Round 2: Turso leaves virtual-table rows out of changes() and
    /// total_changes() (candidate turso-vtab-writes-not-counted-in-changes);
    /// per connection, the total_changes() difference accumulated by
    /// virtual-table writes, and the connection of the current action
    /// when it is a virtual-table write.
    vt_total_offset: Vec<i64>,
    vt_write_conn: Option<usize>,
    /// Per connection: last_insert_rowid() differs since a virtual-table
    /// INSERT (Turso sets it only for INSERT OR REPLACE there).
    vt_rowid: Vec<bool>,
    /// Per connection: a statement read or wrote inside the current
    /// explicit transaction (SQLite has its read snapshot then).
    txn_touched: Vec<bool>,
    /// Per connection: the transaction's first statement used only the
    /// attached database (turso #10825: Turso starts main's read
    /// transaction there too).
    aux_first: Vec<bool>,
    /// A multi-row INSERT into a virtual table ran (candidate
    /// turso-vtab-multirow-insert-keeps-only-last-row).
    vt_multirow: bool,
    /// Per connection: the attached database was used in the transaction.
    aux_touched: Vec<bool>,
    /// Per connection: an IF [NOT] EXISTS DDL ran inside the transaction
    /// (candidate F: Turso takes the write lock even when it is a no-op).
    quiet_ddl_txn: Vec<bool>,
    /// Per connection: DDL on the attached database ran in the transaction
    /// (candidate: Turso takes main's write lock / needs main's snapshot).
    aux_ddl_txn: Vec<bool>,
    /// Slots whose last SQLite step returned SQLITE_BUSY and that were not
    /// reset since (SQLite keeps such a statement's read transaction).
    busy_pending: std::collections::BTreeSet<usize>,
    /// Sticky: a statement ran on a connection while another of its
    /// statements had an unreset SQLITE_BUSY (SQLite keeps that statement's
    /// transaction; its later reset/finalize can roll back the newer
    /// statement's work: round-1 SQLite note).
    sqlite_busy_taint: bool,
    /// Slots whose Turso step returned Busy (candidate
    /// turso-attached-busy-retry-stays-busy).
    turso_busy_slots: std::collections::BTreeSet<usize>,
    /// Round 3: Turso runs in MVCC mode (SQLite in WAL).
    mvcc: bool,
    /// Round 3, MVCC: a write finished in autocommit mode while a sibling
    /// statement on its connection was active (COMPAT.md: its commit is
    /// deferred until the last sibling finishes, and lost if the shared
    /// transaction then ends abnormally). Sticky.
    mvcc_deferred: bool,
    /// Round 3: a checkpoint ran (any mode), and one in RESTART/TRUNCATE mode.
    ckpt_any: bool,
    ckpt_reset: bool,
    /// Round 3, MVCC: per connection, the open transaction began with BEGIN
    /// CONCURRENT; sticky: a statement failed or a rollback ran inside one
    /// (the AUTOINCREMENT value was taken by an inner transaction that
    /// committed: the sequence keeps the gap, see README.md).
    concurrent_txn: Vec<bool>,
    mvcc_seq_gap: bool,
    /// Round 3: the session has TEMP tables on connection 0.
    temp: bool,
    /// Round 3: a trigger writes a virtual table (its rows are missing from
    /// Turso's total_changes(): candidate turso-vtab-writes-not-counted-in-changes).
    vt_trigger: bool,
}

/// Statements after which Turso sets changes() to 0 where SQLite keeps it:
/// CREATE INDEX (candidate D). (CREATE VIRTUAL TABLE / ATTACH / DROP of a
/// virtual table looked alike in round 2 but were after-effects of a
/// virtual-table write; see README.md.)
fn resets_changes(kind: &str, _sql: &str) -> bool {
    kind.starts_with("CREATE INDEX")
}

/// Two shown rows that differ only by an INTEGER against an equal whole
/// REAL in some cells ("2" vs "2.0").
fn int_real_only(a: &str, b: &str) -> bool {
    let strip = |x: &str| x.trim_start_matches("row ").to_string();
    let (a, b) = (strip(a), strip(b));
    let xs: Vec<&str> = a.split('|').collect();
    let ys: Vec<&str> = b.split('|').collect();
    a != b
        && xs.len() == ys.len()
        && xs.iter().zip(&ys).all(|(x, y)| {
            x == y || match (x.parse::<f64>(), y.parse::<f64>()) {
                (Ok(p), Ok(q)) => p == q && (x.contains('.') != y.contains('.')),
                _ => false,
            }
        })
}

/// The connection of an `exec cN: ...` label (0 otherwise).
fn r3_conn_early(label: &str) -> usize {
    label.strip_prefix("exec c").and_then(|x| x.split(':').next()).and_then(|x| x.parse().ok()).unwrap_or(0)
}

fn this_slot_of(a: &Action) -> Option<usize> {
    if let Action::Step { slot, .. } = a {
        Some(*slot)
    } else {
        None
    }
}

fn is_txn_kind(k: &str) -> bool {
    ["BEGIN", "COMMIT", "END", "ROLLBACK", "SAVEPOINT", "RELEASE"].iter().any(|x| k.starts_with(x))
}

/// INSERT/UPDATE/DELETE whose target is a virtual table (vtN).
fn is_vt_write(sql: &str) -> bool {
    let u = sql.trim_start().to_uppercase();
    let target = if u.starts_with("INSERT") || u.starts_with("REPLACE") {
        u.split(" INTO ").nth(1)
    } else if u.starts_with("UPDATE") {
        u.split_whitespace().find(|w| w.starts_with("VT"))
    } else if u.starts_with("DELETE") {
        u.split(" FROM ").nth(1)
    } else {
        None
    };
    target.is_some_and(|t| t.trim_start().starts_with("VT") && (u.starts_with("UPDATE") || t.trim_start()[2..].starts_with(|c: char| c.is_ascii_digit())))
}

/// What an action compared to: `None` when equal.
struct Diff {
    what: String,
    s: String,
    t: String,
    rows: bool,
}

impl Runner<'_> {
    fn turso(&mut self) -> &mut Turso {
        self.t.as_mut().expect("turso engine")
    }

    fn sqlite_readonly(&self, slot: usize) -> bool {
        // The SQLite statement for the slot, through a probe prepare: the
        // engine keeps its pointers private, so ask by SQL.
        let m = &self.meta[&slot];
        let u = m.sql.trim_start().to_uppercase();
        !(u.starts_with("INSERT")
            || u.starts_with("UPDATE")
            || u.starts_with("DELETE")
            || u.starts_with("REPLACE")
            || u.starts_with("CREATE")
            || u.starts_with("DROP")
            || u.starts_with("ALTER")
            || u.starts_with("ROLLBACK"))
    }

    /// Same-connection statements suspended while `writer_sql` writes.
    fn note_write(&mut self, conn: usize, slot: Option<usize>, sql: &str) {
        let u = sql.trim_start().to_uppercase();
        let is_txn = ["BEGIN", "COMMIT", "END", "ROLLBACK", "SAVEPOINT", "RELEASE"].iter().any(|k| u.starts_with(k));
        let writes = !is_txn && !u.starts_with("SELECT") && !u.starts_with("WITH");
        let rollback = u.starts_with("ROLLBACK");
        let tabs = tables_of(sql);
        let mut kinds = Vec::new();
        for (k, m) in self.meta.iter_mut() {
            if Some(*k) == slot || m.conn != conn || !m.suspended {
                continue;
            }
            kinds.push(if m.writer { if m.returning { "suspended writer RETURNING" } else { "suspended writer" } } else { "suspended read" });
            let shared = tables_of(&m.sql).iter().any(|x| tabs.contains(x));
            // Triggers and foreign keys write other tables: any write counts.
            if rollback || (writes && (shared || self.cascades)) {
                m.overlapped = true;
            }
        }
        if is_txn || writes {
            for k in kinds {
                self.stats.add(format!("interleave: {} over {k}", stmt_kind(sql).replace(" RETURNING", "")), 1);
            }
        }
    }

    fn log(&mut self, s: String) {
        if self.verbose {
            println!("{s}");
        }
        self.log.push(s);
    }

    fn compare_evs(&mut self, slot: Option<usize>, s: &[Ev], t: &[Ev], returning: bool) -> Option<Diff> {
        for (k, (a, b)) in s.iter().zip(t.iter()).enumerate() {
            let same = match (a, b) {
                (Ev::Row(x), Ev::Row(y)) => {
                    if returning {
                        if let Some(sl) = slot {
                            let m = self.meta.get_mut(&sl).unwrap();
                            m.rows_s.push(x.clone());
                            m.rows_t.push(y.clone());
                        }
                        true
                    } else {
                        cells_equal(x, y)
                    }
                },
                (Ev::Err(c, _), Ev::Err(d, _)) => c == d,
                (Ev::Panic(_), _) | (_, Ev::Panic(_)) => false,
                _ => a == b,
            };
            if !same {
                return Some(Diff {
                    what: format!("event {k}"),
                    s: show_ev(a),
                    t: show_ev(b),
                    rows: matches!((a, b), (Ev::Row(_), Ev::Row(_))),
                });
            }
        }
        if s.len() != t.len() {
            let n = s.len().min(t.len());
            return Some(Diff {
                what: format!("event count {} vs {}", s.len(), t.len()),
                s: s.get(n).map_or("(end)".into(), show_ev),
                t: t.get(n).map_or("(end)".into(), show_ev),
                rows: false,
            });
        }
        None
    }

    /// RETURNING rows gathered for `slot`, compared as multisets (SQLite
    /// leaves their order unspecified) once the statement ends.
    fn returning_end(&mut self, slot: usize) -> Option<Diff> {
        let m = self.meta.get_mut(&slot)?;
        let mut a: Vec<Vec<Cell>> = std::mem::take(&mut m.rows_s).iter().map(|r| canonical(r)).collect();
        let mut b: Vec<Vec<Cell>> = std::mem::take(&mut m.rows_t).iter().map(|r| canonical(r)).collect();
        let key = |r: &Vec<Cell>| show_cells(r);
        a.sort_by_key(key);
        b.sort_by_key(key);
        if a.len() == b.len() && a.iter().zip(&b).all(|(x, y)| cells_equal(x, y)) {
            None
        } else {
            Some(Diff {
                what: "RETURNING rows (as multisets)".into(),
                s: a.iter().map(|r| show_cells(r)).collect::<Vec<_>>().join(" ; "),
                t: b.iter().map(|r| show_cells(r)).collect::<Vec<_>>().join(" ; "),
                rows: true,
            })
        }
    }

    fn compare_states(&mut self) -> Option<Diff> {
        let n = self.s.conns();
        for c in 0..n {
            let a = self.s.state(c);
            let t = self.t.as_ref().unwrap();
            let mut b = guarded(|| Some(t.state(c)), |_| None)?;
            if a.last_rowid == b.last_rowid {
                self.window_rowid[c] = false;
                self.vt_rowid[c] = false;
            }
            if a.last_rowid != b.last_rowid && (self.vt_write_conn == Some(c) || self.vt_rowid[c]) {
                self.stats.add("tolerated: last_insert_rowid() after a virtual-table INSERT (candidate)", 1);
                self.vt_rowid[c] = true;
                b.last_rowid = a.last_rowid;
            }
            if a.last_rowid != b.last_rowid && self.window_rowid[c] {
                self.stats.add("tolerated: last_insert_rowid() after a window query (#8698)", 1);
                b.last_rowid = a.last_rowid;
            }
            b.total_changes += self.vt_total_offset[c];
            if self.vt_write_conn == Some(c) && (a.changes != b.changes || a.total_changes != b.total_changes) {
                self.stats.add("tolerated: changes()/total_changes() after a virtual-table write (candidate)", 1);
                self.vt_total_offset[c] += a.total_changes - b.total_changes;
                b.total_changes = a.total_changes;
                b.changes = a.changes;
            }
            if a.changes != b.changes && self.index_changes.get(c).copied().unwrap_or(false) {
                self.stats.add("tolerated: changes() after CREATE INDEX or a BUSY DML", 1);
                b.changes = a.changes;
            }
            if a != b {
                let field = if a.autocommit != b.autocommit {
                    "autocommit"
                } else if a.changes != b.changes {
                    "changes()"
                } else if a.total_changes != b.total_changes {
                    "total_changes()"
                } else {
                    "last_insert_rowid()"
                };
                return Some(Diff { what: format!("state {field} c{c}"), s: show_state(&a), t: show_state(&b), rows: false });
            }
        }
        None
    }

    /// One statement step on both engines, with auto-reset after done or
    /// an error (as `sqlite3_step` does since 3.7.0).
    fn step_both(&mut self, slot: usize) -> (Ev, Ev) {
        let s = self.s.step(slot);
        let t = {
            let tt = self.turso();
            guarded(|| tt.step(slot), Ev::Panic)
        };
        (s, t)
    }

    fn run_action(&mut self, a: &Action) -> Result<(), (Diff, String)> {
        let label = gen::render_action(a);
        self.vt_write_conn = match a {
            Action::Exec { conn, sql } if is_vt_write(sql) => Some(*conn),
            Action::Step { slot, .. } => self.meta.get(slot).filter(|m| is_vt_write(&m.sql)).map(|m| m.conn),
            _ => None,
        };
        let auto_before: Vec<bool> = (0..self.s.conns()).map(|c| self.s.state(c).autocommit).collect();
        self.last_slot = None;
        self.stats.add(format!("action: {}", a.kind()), 1);
        let (sev, tev, slot, returning, kind): (Vec<Ev>, Vec<Ev>, Option<usize>, bool, String) = match a {
            Action::Prepare { slot, conn, sql } => {
                if let Some(d) = self.end_statement(*slot) {
                    return Err((d, label));
                }
                let mut s = self.s.prepare(*slot, *conn, sql);
                let t = {
                    let tt = self.turso();
                    guarded(|| tt.prepare(*slot, *conn, sql), Ev::Panic)
                };
                self.epochs.insert(*slot, self.schema_epoch);
                if let (Ev::Ok, Ev::Err(c, m)) = (&s, &t) {
                    // Turso checks the schema at prepare, SQLite at the first
                    // step (it prepares against its cached schema).
                    if !unsupported(m) {
                        if let Ev::Err(c2, _) = self.s.step(*slot) {
                            if c2 == *c {
                                self.stats.add("tolerated: error at prepare (Turso) vs first step (SQLite)", 1);
                                s = t.clone();
                            }
                        }
                    }
                }
                if s == Ev::Ok && t == Ev::Ok {
                    let mut m = SlotMeta { sql: sql.clone(), conn: *conn, returning: sql.to_uppercase().contains(" RETURNING "), ..SlotMeta::default() };
                    self.meta.insert(*slot, m.clone());
                    m.writer = !self.sqlite_readonly(*slot);
                    self.meta.insert(*slot, m);
                } else {
                    // One side has no statement: keep both alike.
                    self.s.finalize(*slot);
                    let tt = self.turso();
                    guarded(|| tt.finalize(*slot), |_| ());
                }
                (vec![s], vec![t], Some(*slot), false, format!("prepare {}", stmt_kind(sql)))
            },
            Action::Bind { slot, idx, val } => {
                let s = self.s.bind(*slot, *idx, val);
                let t = {
                    let tt = self.turso();
                    guarded(|| tt.bind(*slot, *idx, val), Ev::Panic)
                };
                (vec![s], vec![t], Some(*slot), false, "bind".into())
            },
            Action::Step { slot, n } => {
                let Some(m) = self.meta.get(slot).cloned() else {
                    return Ok(());
                };
                let mut sv = Vec::new();
                let mut tv = Vec::new();
                if m.halted {
                    // Auto-reset, as sqlite3_step does after done or an error.
                    self.s.reset(*slot);
                    let tt = self.turso();
                    let r = guarded(|| tt.reset(*slot), Ev::Panic);
                    if r != Ev::Ok {
                        sv.push(Ev::Ok);
                        tv.push(r);
                    }
                    let mm = self.meta.get_mut(slot).unwrap();
                    mm.halted = false;
                    mm.overlapped = false;
                }
                self.last_slot = Some(*slot);
                if self.epochs.get(slot).is_some_and(|e| *e < self.schema_epoch) && self.meta.iter().any(|(k, o)| k != slot && o.conn == m.conn && o.suspended) {
                    self.known.push("E");
                }
                if m.writer {
                    self.note_write(m.conn, Some(*slot), &m.sql);
                }
                if !tv.is_empty() {
                    (sv, tv, Some(*slot), m.returning, format!("step {}", stmt_kind(&m.sql)))
                } else {
                    for _ in 0..*n {
                        let (s, t) = self.step_both(*slot);
                        let stop = !matches!(s, Ev::Row(_)) || !matches!(t, Ev::Row(_));
                        let mm = self.meta.get_mut(slot).unwrap();
                        mm.stepped = true;
                        mm.suspended = matches!(s, Ev::Row(_));
                        mm.halted = matches!(s, Ev::Done) || matches!(&s, Ev::Err(c, _) if *c != Class::Busy);
                        if let Ev::Err(c, _) = &s {
                            self.stats.add(format!("sqlite error class: {c:?}"), 1);
                        }
                        if matches!(&t, Ev::Err(Class::Busy, _)) {
                            self.turso_busy_slots.insert(*slot);
                        }
                        if matches!(&s, Ev::Err(Class::Busy, _)) {
                            self.busy_pending.insert(*slot);
                        } else {
                            self.busy_pending.remove(slot);
                        }
                        sv.push(s);
                        tv.push(t);
                        if stop {
                            break;
                        }
                    }
                    if self.meta[slot].suspended {
                        let w = if m.writer { if m.returning { "writer RETURNING" } else { "writer" } } else { "read" };
                        self.stats.add(format!("left suspended: {w}"), 1);
                    }
                    (sv, tv, Some(*slot), m.returning, format!("step {}", stmt_kind(&m.sql)))
                }
            },
            Action::Reset { slot } => {
                let d = self.end_statement(*slot);
                if let Some(d) = d {
                    return Err((d, label));
                }
                self.abandon_check(*slot);
                self.note_end(*slot);
                self.busy_pending.remove(slot);
                let s = self.s.reset(*slot);
                let t = {
                    let tt = self.turso();
                    guarded(|| tt.reset(*slot), Ev::Panic)
                };
                if let Some(m) = self.meta.get_mut(slot) {
                    m.suspended = false;
                    m.halted = false;
                    m.overlapped = false;
                }
                let k = self.meta.get(slot).map_or(String::new(), |m| stmt_kind(&m.sql));
                (vec![s], vec![t], Some(*slot), false, format!("reset {k}"))
            },
            Action::Finalize { slot } => {
                if let Some(d) = self.end_statement(*slot) {
                    return Err((d, label));
                }
                self.abandon_check(*slot);
                self.note_end(*slot);
                self.busy_pending.remove(slot);
                self.s.finalize(*slot);
                let tt = self.turso();
                let t = guarded(
                    || {
                        tt.finalize(*slot);
                        Ev::Ok
                    },
                    Ev::Panic,
                );
                let k = self.meta.remove(slot).map_or(String::new(), |m| stmt_kind(&m.sql));
                (vec![Ev::Ok], vec![t], Some(*slot), false, format!("finalize {k}"))
            },
            Action::Exec { conn, sql } => {
                self.note_write(*conn, None, sql);
                let (sv, tv) = self.exec_both(*conn, sql);
                let returning = sql.to_uppercase().contains(" RETURNING ");
                (sv, tv, None, returning, format!("exec {}", stmt_kind(sql)))
            },
        };
        self.log(format!("{label}\n    sqlite: {}\n    turso:  {}", sev.iter().map(show_ev).collect::<Vec<_>>().join(", "), tev.iter().map(show_ev).collect::<Vec<_>>().join(", ")));
        self.log_vt_events();
        // Round 3: checkpoint rows (busy, log frames, checkpointed frames)
        // count each engine's own WAL frames: only the result kind is
        // compared.
        let ckpt_sql = match a {
            Action::Exec { sql, .. } => sql.clone(),
            Action::Step { slot, .. } => self.meta.get(slot).map_or(String::new(), |m| m.sql.clone()),
            _ => String::new(),
        };
        let (sev, tev) = if ckpt_sql.contains("wal_checkpoint") {
            self.stats.add("checkpoint statements (rows not compared)", 1);
            let blank = |v: Vec<Ev>| v.into_iter().map(|e| if let Ev::Row(_) = e { Ev::Row(vec![]) } else { e }).collect::<Vec<_>>();
            let (sev, tev) = (blank(sev), blank(tev));
            // Turso refuses an explicit checkpoint while another statement
            // of the connection is active (connection.rs
            // begin_explicit_checkpoint, by design); a checkpoint changes no
            // data, so the session goes on.
            if tev.iter().any(|e| matches!(e, Ev::Err(Class::Busy, m) if m.contains("cannot checkpoint while another statement is active"))) {
                self.stats.add("tolerated: Turso refuses a checkpoint while another statement is active (by design)", 1);
                (sev.clone(), sev)
            } else {
                (sev, tev)
            }
        } else {
            (sev, tev)
        };
        let mut diff = None;
        if let Action::Exec { .. } = a {
            // Exec rows: compare RETURNING rows as multisets.
            if returning {
                let rs: Vec<Vec<Cell>> = sev.iter().filter_map(|e| if let Ev::Row(r) = e { Some(r.clone()) } else { None }).collect();
                let rt: Vec<Vec<Cell>> = tev.iter().filter_map(|e| if let Ev::Row(r) = e { Some(r.clone()) } else { None }).collect();
                self.meta.insert(HIDDEN, SlotMeta { rows_s: rs, rows_t: rt, ..SlotMeta::default() });
                diff = self.compare_evs(None, &sev, &tev, true).or_else(|| self.returning_end(HIDDEN));
                self.meta.remove(&HIDDEN);
            } else {
                diff = self.compare_evs(None, &sev, &tev, false);
            }
        } else {
            diff = diff.or_else(|| self.compare_evs(slot, &sev, &tev, returning));
            if diff.is_none() {
                if let (Action::Step { slot, .. }, Some(Ev::Done)) = (a, sev.last()) {
                    diff = self.returning_end(*slot);
                }
            }
        }
        self.after_action(a, &sev, &auto_before);
        // Which connection finished which statement (for changes()).
        let finished = match a {
            Action::Exec { conn, sql } => Some((*conn, sql.clone())),
            Action::Step { slot, .. } if matches!(sev.last(), Some(Ev::Done) | Some(Ev::Err(..))) => self.meta.get(slot).map(|m| (m.conn, m.sql.clone())),
            _ => None,
        };
        if let Some((c, sql)) = finished {
            let k = stmt_kind(&sql);
            if is_vt_write(&sql) {
                // Turso leaves virtual-table rows out of changes() (candidate).
                self.index_changes[c] = true;
            } else if resets_changes(&k, &sql) && matches!(sev.last(), Some(Ev::Done)) {
                self.index_changes[c] = true;
            } else if ["INSERT", "UPDATE", "DELETE", "REPLACE"].iter().any(|x| k.starts_with(x)) {
                match sev.last() {
                    Some(Ev::Done) => self.index_changes[c] = false,
                    // SQLite sets changes() to 0 when a DML statement fails
                    // with SQLITE_BUSY; Turso keeps the previous value.
                    Some(Ev::Err(Class::Busy, _)) => self.index_changes[c] = true,
                    _ => {},
                }
            }
        }
        let diff = diff.or_else(|| self.compare_states());
        match diff {
            None => Ok(()),
            Some(d) => Err((d, kind)),
        }
    }

    /// The appvt hook calls of both engines since the last action.
    fn log_vt_events(&mut self) {
        let a = self.s.vt_events();
        let b = self.t.as_ref().map(|t| t.vt_events()).unwrap_or_default();
        if !a.is_empty() || !b.is_empty() {
            self.stats.add("vt hook calls (sqlite)", a.len() as u64);
            let tag = if a == b { "same" } else { "DIFFER" };
            // Candidate turso-vtab-rolled-back-by-failed-statement: Turso calls
            // the module's rollback when any statement halts with an error.
            if b.iter().any(|e| e.ends_with(":rollback")) && !a.iter().any(|e| e.ends_with(":rollback")) {
                self.known.push("VTROLLBACK");
            }
            self.log(format!("    vt hooks ({tag}): sqlite [{}] turso [{}]", a.join(", "), b.join(", ")));
        }
    }

    /// A statement on a connection reset or finalized while suspended.
    fn note_end(&mut self, slot: usize) {
        let Some(m) = self.meta.get(&slot) else { return };
        if !m.suspended && !(m.stepped && !m.halted) {
            return;
        }
        let auto = self.s.state(m.conn).autocommit;
        if !auto {
            self.known.push("B");
        } else if self.meta.iter().any(|(k, o)| *k != slot && o.conn == m.conn && o.suspended) {
            self.known.push("A");
        }
        // changes(): SQLite sets it when a DML statement halts here.
        if m.writer {
            self.index_changes[m.conn] = true;
        }
    }

    fn after_action(&mut self, a: &Action, sev: &[Ev], auto_before: &[bool]) {
        let (conn, sql) = match a {
            Action::Exec { conn, sql } => (*conn, sql.clone()),
            Action::Step { slot, .. } => match self.meta.get(slot) {
                Some(m) => (m.conn, m.sql.clone()),
                None => return,
            },
            _ => {
                for c in 0..self.s.conns() {
                    if self.s.state(c).autocommit {
                        self.sp_begin[c] = false;
                        self.txn_touched[c] = false;
                        self.aux_first[c] = false;
                self.aux_touched[c] = false;
                self.quiet_ddl_txn[c] = false;
                self.aux_ddl_txn[c] = false;
                    }
                }
                return;
            },
        };
        let k = stmt_kind(&sql);
        let done = matches!(sev.last(), Some(Ev::Done));
        if self.mvcc && done && auto_before[conn] && !is_txn_kind(&k) && !k.starts_with("SELECT") && !k.starts_with("WITH") {
            let this = this_slot_of(a);
            if self.meta.iter().any(|(kk, o)| Some(*kk) != this && o.conn == conn && (o.suspended || (o.stepped && !o.halted))) {
                self.mvcc_deferred = true;
                self.stats.add("mvcc: write finished with a sibling statement active", 1);
            }
        }
        if k == "ROLLBACK TO" {
            self.saw_rollback_to = true;
        }
        if self.mvcc {
            if sql.starts_with("BEGIN CONCURRENT") && done {
                self.concurrent_txn[conn] = true;
            }
            let failed_stmt = matches!(sev.last(), Some(Ev::Err(..)));
            if self.concurrent_txn[conn] && (k.starts_with("ROLLBACK") || failed_stmt) {
                self.mvcc_seq_gap = true;
            }
            if self.s.state(conn).autocommit && !sql.starts_with("BEGIN CONCURRENT") {
                self.concurrent_txn[conn] = false;
            }
        }
        if self.busy_pending.iter().any(|b| Some(*b) != this_slot_of(a) && self.meta.get(b).is_some_and(|m| m.conn == conn)) {
            self.sqlite_busy_taint = true;
        }
        // turso #9323/#9364: a failed statement, ROLLBACK TO or ROLLBACK
        // stops the connection's other open SELECTs.
        let failed = matches!(sev.last(), Some(Ev::Err(c, _)) if *c != Class::Busy);
        let this_slot = if let Action::Step { slot, .. } = a { Some(*slot) } else { None };
        if (failed || k == "ROLLBACK TO" || k == "ROLLBACK") && self.meta.iter().any(|(kk, o)| Some(*kk) != this_slot && o.conn == conn && o.suspended) {
            self.known.push("9323");
        }
        if !self.s.state(conn).autocommit && ["CREATE", "DROP", "ALTER"].iter().any(|x| k.starts_with(x)) && sql.contains("aux.") {
            self.aux_ddl_txn[conn] = true;
        }
        if sql.contains(" OVER ") {
            self.window_rowid[conn] = true;
        }
        // A transaction end while a statement is suspended: Turso ends the
        // read transaction under it (candidate A, session-commit.txt).
        if self.s.state(conn).autocommit && !auto_before[conn] && ["COMMIT", "END", "RELEASE", "ROLLBACK"].iter().any(|x| k == *x) && self.meta.values().any(|o| o.conn == conn && o.suspended) {
            self.known.push("A");
            // Round 3 candidate turso-mvcc-commit-with-suspended-select.
            if self.mvcc {
                self.known.push("MVCCCOMMIT");
            }
        }
        if done && (k.starts_with("CREATE") || k.starts_with("DROP") || k.starts_with("ALTER")) {
            self.schema_epoch += 1;
        }
        if sql.contains("wal_checkpoint") {
            self.ckpt_any = true;
            if sql.contains("RESTART") || sql.contains("TRUNCATE") {
                self.ckpt_reset = true;
            }
        }
        // turso-attached-busy-retry-stays-busy's mechanism: after Busy on an
        // attached database inside a transaction, Turso keeps that
        // database's read snapshot (later reads miss newer commits).
        if matches!(sev.last(), Some(Ev::Err(Class::Busy, _))) && sql.contains("aux") && !self.s.state(conn).autocommit {
            self.known.push("AUXBUSY");
        }
        // Round 3 candidate turso-vtab-registry-stale: a vtab renamed, or
        // dropped inside a transaction (which may roll back).
        let vt_named = tables_of(&sql).iter().any(|t| t.starts_with("vt"));
        if done && vt_named && ((k == "ALTER" && sql.contains(" RENAME ")) || (k == "DROP TABLE" && !self.s.state(conn).autocommit)) {
            self.known.push("VTREG");
        }
        let auto_after = self.s.state(conn).autocommit;
        if k == "SAVEPOINT" && auto_before[conn] && !auto_after {
            self.sp_begin[conn] = true;
        }
        // turso #11394 also when BEGIN (deferred) came first: SQLite has no
        // snapshot until the first read, Turso takes it at SAVEPOINT.
        if k == "SAVEPOINT" && !auto_after && (!self.txn_touched[conn] || (self.attach && !self.aux_touched[conn])) {
            self.sp_begin[conn] = true;
        }
        if !auto_after && !is_txn_kind(&k) {
            if tables_of(&sql).iter().any(|t| t.starts_with('u')) {
                self.aux_touched[conn] = true;
            }
            let failed_ddl = matches!(sev.last(), Some(Ev::Err(..))) && ["CREATE", "DROP", "ALTER"].iter().any(|x| k.starts_with(x));
            if sql.contains("IF EXISTS") || sql.contains("IF NOT EXISTS") || failed_ddl {
                self.quiet_ddl_txn[conn] = true;
            }
        }
        if sql.contains(" vt") && sql.contains("), (") && is_vt_write(&sql) {
            self.vt_multirow = true;
        }
        let is_txn = ["BEGIN", "COMMIT", "END", "ROLLBACK", "SAVEPOINT", "RELEASE"].iter().any(|x| k.starts_with(x));
        let immediate = k == "BEGIN" && (sql.to_uppercase().contains("IMMEDIATE") || sql.to_uppercase().contains("EXCLUSIVE"));
        let attach_like = k.starts_with("ATTACH") || k.starts_with("DETACH") || k.starts_with("PRAGMA");
        if !auto_after && !attach_like && (immediate || (!is_txn && !self.txn_touched[conn])) {
            if !immediate {
                let tabs = tables_of(&sql);
                if !tabs.is_empty() && tabs.iter().all(|t| t.starts_with('u') || t.starts_with('w')) {
                    self.aux_first[conn] = true;
                }
            }
            self.txn_touched[conn] = true;
        }
        if done && auto_after {
            if let Action::Step { slot, .. } = a {
                if self.meta.iter().any(|(kk, o)| kk != slot && o.conn == conn && o.suspended) {
                    self.known.push("A");
                }
            }
            // Round 2: a one-shot statement finishing counts too (A's
            // trigger is any sibling statement end), two connections only.
            if matches!(a, Action::Exec { .. }) && self.s.conns() == 2 && self.meta.values().any(|o| o.conn == conn && o.suspended) {
                self.known.push("A");
            }
        }
        for c in 0..self.s.conns() {
            if self.s.state(c).autocommit {
                self.sp_begin[c] = false;
                self.txn_touched[c] = false;
                self.aux_first[c] = false;
                self.aux_touched[c] = false;
                self.quiet_ddl_txn[c] = false;
                self.aux_ddl_txn[c] = false;
            }
        }
    }

    /// COMPAT.md's rollback-only rule: resetting or dropping a write
    /// statement that has not finished, inside an explicit transaction.
    fn abandon_check(&mut self, slot: usize) {
        if let Some(m) = self.meta.get(&slot) {
            if m.writer && m.stepped && !m.halted && !self.s.state(m.conn).autocommit {
                self.tainted = Some(format!("{} reset/finalized before done inside a transaction", stmt_kind(&m.sql)));
                self.stats.add("abandoned writer in transaction", 1);
            }
        }
    }

    /// A RETURNING statement reset, finalized or replaced before it
    /// finished: which rows came first is unspecified, so only the number
    /// of rows returned is compared (round 2 fix; the full multisets are
    /// compared when the statement reaches done).
    fn end_statement(&mut self, slot: usize) -> Option<Diff> {
        let m = self.meta.get_mut(&slot)?;
        if !m.returning {
            return None;
        }
        let a = std::mem::take(&mut m.rows_s);
        let b = std::mem::take(&mut m.rows_t);
        if a.len() == b.len() {
            return None;
        }
        Some(Diff { what: format!("RETURNING row count of an unfinished statement {} vs {}", a.len(), b.len()), s: a.len().to_string(), t: b.len().to_string(), rows: true })
    }

    fn exec_both(&mut self, conn: usize, sql: &str) -> (Vec<Ev>, Vec<Ev>) {
        let run = |e: &mut dyn Engine| -> Vec<Ev> {
            let mut v = vec![e.prepare(HIDDEN, conn, sql)];
            if v[0] == Ev::Ok {
                // Round 3: to the end (a bulk DELETE ... RETURNING returns
                // more than STEP_ALL rows; a partial multiset is not
                // comparable).
                for _ in 0..100_000 {
                    let r = e.step(HIDDEN);
                    let stop = !matches!(r, Ev::Row(_));
                    v.push(r);
                    if stop {
                        break;
                    }
                }
            }
            e.finalize(HIDDEN);
            v
        };
        let mut sv = run(&mut self.s);
        if let Some(Ev::Err(c, _)) = sv.last() {
            self.stats.add(format!("sqlite error class: {c:?}"), 1);
        }
        let tt = self.t.as_mut().unwrap();
        let tv = guarded(|| run(tt), |m| vec![Ev::Panic(m)]);
        // Turso checks the schema at prepare, SQLite at the first step
        // (where it may meet SQLITE_BUSY before its schema check).
        if let ([Ev::Ok, Ev::Err(c, _)], [Ev::Err(d, m)]) = (sv.as_slice(), tv.as_slice()) {
            let schema_msg = m.contains("already exists") || m.contains("no such table");
            if (c == d || (*c == Class::Busy && schema_msg)) && !unsupported(m) {
                self.stats.add("tolerated: error at prepare (Turso) vs first step (SQLite)", 1);
                sv = tv.clone();
            }
        }
        (sv, tv)
    }

    fn classify(&self, d: &Diff, kind: &str, label: &str) -> Divergence {
        let detail = format!("at: {label}\n  {}\n  sqlite: {}\n  turso:  {}", d.what, d.s, d.t);
        if d.t.starts_with("PANIC") {
            let m = d.t.trim_start_matches("PANIC ");
            if m.contains("shared autocommit transaction needed full rollback") {
                return Divergence { verdict: Verdict::Known, signature: format!("known window-select-reset panic: {}", normalize(m)), detail };
            }
            if m.contains("NotExists on unexpected cursor") {
                return Divergence { verdict: Verdict::Known, signature: "known vt-update-rowid panic (NotExists on a virtual-table cursor)".into(), detail };
            }
            if m.contains("transaction should exist while its cursor is active") && self.mvcc {
                let how = if self.known.contains(&"MVCCCOMMIT") { "transaction ended with the SELECT suspended" } else { "sibling write finished with the SELECT suspended" };
                return Divergence { verdict: Verdict::Known, signature: format!("known #9327/#9619 MVCC panic (read continues after its transaction ends: {how})"), detail };
            }
            if (m.contains("Could not find virtual table") || m.contains("vtab must exist")) && self.known.contains(&"VTREG") {
                return Divergence { verdict: Verdict::Known, signature: "known vt-registry panic (vtab write after RENAME or DROP+ROLLBACK)".into(), detail };
            }
            if m.contains("Could not find virtual table for VBegin") || m.contains("vtab must exist") {
                return Divergence { verdict: Verdict::Known, signature: "known vt-other-connection panic".into(), detail };
            }
            return Divergence { verdict: Verdict::Finding, signature: format!("panic: {}", normalize(m)), detail };
        }
        let t_err = d.t.split_once(": ").map(|x| x.1).unwrap_or("");
        let s_is_err = d.s.split_once(':').is_some_and(|(c, _)| {
            ["Error", "Busy", "Locked", "Constraint", "Abort", "ReadOnly", "Corrupt", "Full", "Schema", "Interrupt", "Misuse", "Range", "TooBig", "Other"].contains(&c)
        });
        if !s_is_err && unsupported(t_err) {
            return Divergence { verdict: Verdict::Skip, signature: format!("turso unsupported: {}", normalize(t_err)), detail };
        }
        if t_err.to_lowercase().contains("statements in progress") {
            return Divergence { verdict: Verdict::Skip, signature: format!("documented: same-connection writer busy ({kind})"), detail };
        }
        let pre_sig_early = {
            let k = |x: &str| x.split(':').next().unwrap_or("").split_whitespace().next().unwrap_or("").to_string();
            if d.rows { format!("{kind}: rows differ") } else if d.what.starts_with("state") { format!("{kind}: state {} differs", d.what.split_whitespace().nth(1).unwrap_or("")) } else { format!("{kind}: sqlite {} / turso {}", k(&d.s), k(&d.t)) }
        };
        if let Some(why) = &self.tainted {
            return Divergence { verdict: Verdict::Skip, signature: "documented: rollback-only after abandoned writer".into(), detail: format!("{detail}\n  taint: {why}") };
        }
        if (t_err.contains("unsafe use of virtual table") && !s_is_err) || self.log.iter().any(|l| l.contains("unsafe use of virtual table")) {
            return Divergence { verdict: Verdict::Skip, signature: "turso rejects a virtual table written by a trigger (unsafe use; SQLite allows innocuous vtabs)".into(), detail };
        }
        if self.known.contains(&"MVCCCOMMIT") {
            return Divergence { verdict: Verdict::Known, signature: format!("known mvcc-commit-suspended (transaction ended with a SELECT suspended): {pre_sig_early}"), detail };
        }
        if self.mvcc && self.s.conns() == 1 && t_err.contains("Busy") && !s_is_err && ["CREATE", "DROP", "ALTER"].iter().any(|x| kind.contains(x)) {
            return Divergence { verdict: Verdict::Known, signature: format!("known mvcc-concurrent-ddl (DDL after a write in BEGIN CONCURRENT is Busy): {pre_sig_early}"), detail };
        }
        let s_class = d.s.split(':').next().unwrap_or("");
        let t_class = d.t.split(':').next().unwrap_or("");
        if label.starts_with("exec c") && label.contains(": BEGIN") && s_class == "Error" && t_class == "Busy" && !self.s.state(r3_conn_early(label)).autocommit {
            return Divergence { verdict: Verdict::Unspecified, signature: format!("error precedence: nested BEGIN (SQLite: within a transaction; Turso: Busy): {pre_sig_early}"), detail };
        }
        if self.mvcc && self.s.conns() == 2 && t_class == "Busy" && s_class != "Busy" && !s_is_err && !t_err.contains("statements in progress") {
            return Divergence { verdict: Verdict::Unspecified, signature: format!("MVCC two connections: Turso refuses the write upgrade after a commit since the snapshot (SQLite: identical-content overwrite writes no frame): {pre_sig_early}"), detail };
        }
        if self.mvcc && self.s.conns() == 2 && s_class == "Busy" && t_class != "Busy" && !t_class.starts_with("Error") {
            return Divergence { verdict: Verdict::Unspecified, signature: format!("MVCC two connections: SQLite BUSY where Turso's MVCC transaction saw no conflicting commit: {pre_sig_early}"), detail };
        }
        if self.mvcc_seq_gap {
            return Divergence { verdict: Verdict::Skip, signature: "MVCC: AUTOINCREMENT value kept after a rollback or failed statement in BEGIN CONCURRENT (inner sequence transaction; gaps permitted by autoinc.html)".into(), detail };
        }
        if self.mvcc_deferred {
            return Divergence { verdict: Verdict::Skip, signature: "documented: MVCC shared transaction (write committed only when the last sibling statement finishes)".into(), detail };
        }
        let row_or_done = |x: &str| x.starts_with("row") || x == "done" || x == "(end)";
        let s_class = d.s.split(':').next().unwrap_or("");
        let t_class = d.t.split(':').next().unwrap_or("");
        let pre_sig = if d.rows {
            format!("{kind}: rows differ")
        } else if d.what.starts_with("state") {
            format!("{kind}: state {} differs", d.what.split_whitespace().nth(1).unwrap_or(""))
        } else {
            let k = |x: &str| x.split(':').next().unwrap_or("").split_whitespace().next().unwrap_or("").to_string();
            format!("{kind}: sqlite {} / turso {}", k(&d.s), k(&d.t))
        };
        if s_class == "Abort" {
            return Divergence { verdict: Verdict::Unspecified, signature: format!("sqlite ABORT_ROLLBACK of a pending statement: {pre_sig}"), detail };
        }
        // Round 3 rules (checkpoints, DETACH).
        let r3_sql = match label.split_once(": ") {
            Some((h, q)) if h.starts_with("exec") => q.to_string(),
            _ => self.last_slot.and_then(|k| self.meta.get(&k)).map_or(String::new(), |m| m.sql.clone()),
        };
        let r3_conn = match label.split_whitespace().nth(1) {
            Some(c) if c.starts_with('c') => c[1..].trim_end_matches(':').parse().unwrap_or(0),
            _ => self.last_slot.and_then(|k| self.meta.get(&k)).map_or(0, |m| m.conn),
        };
        let in_txn = r3_conn < self.s.conns() && !self.s.state(r3_conn).autocommit;
        if r3_sql.contains("wal_checkpoint") && (s_class == "Locked" || t_class == "Locked") {
            if self.sp_begin.get(r3_conn).copied().unwrap_or(false) {
                return Divergence { verdict: Verdict::Known, signature: format!("known #11394 (SAVEPOINT snapshot; checkpoint inside the transaction): {pre_sig}"), detail };
            }
            if in_txn {
                return Divergence { verdict: Verdict::Unspecified, signature: format!("checkpoint inside a transaction: which databases the transaction holds differs: {pre_sig}"), detail };
            }
        }
        let quiet_any = self.log.iter().any(|l| (l.starts_with("exec") || l.starts_with("prepare")) && (l.contains("IF EXISTS") || l.contains("IF NOT EXISTS")));
        if r3_sql.starts_with("DETACH") && d.s.contains("is locked") && !t_class.starts_with("Error") && (in_txn || quiet_any) && (quiet_any || self.quiet_ddl_txn.get(r3_conn).copied().unwrap_or(false)) {
            return Divergence { verdict: Verdict::Unspecified, signature: format!("SQLite: an IF [NOT] EXISTS DDL in the transaction read every attached database (DETACH refused): {pre_sig}"), detail };
        }
        if s_class == "Busy" && !t_class.starts_with("Busy") && !t_class.starts_with("Error") && self.ckpt_reset && self.s.conns() == 2 {
            return Divergence { verdict: Verdict::Unspecified, signature: format!("SQLite BUSY after a RESTART/TRUNCATE checkpoint with another connection reading: {pre_sig}"), detail };
        }
        if t_class == "Busy" && s_class != "Busy" && self.ckpt_any && self.s.conns() == 2 && !t_err.contains("statements in progress") {
            return Divergence { verdict: Verdict::Known, signature: format!("known ckpt-noop-stale (r3 candidate: after a checkpoint, a no-op write stales other snapshots): {pre_sig}"), detail };
        }
        if self.known.contains(&"AUXBUSY") {
            return Divergence { verdict: Verdict::Known, signature: format!("known aux-busy-retry (attached database snapshot kept after Busy): {pre_sig}"), detail };
        }
        let stmt_sql = format!("{label} {}", self.last_slot.and_then(|k| self.meta.get(&k)).map_or("", |m| m.sql.as_str()));
        let quiet_ddl = stmt_sql.contains("IF EXISTS") || stmt_sql.contains("IF NOT EXISTS");
        if s_class == "Busy" && d.s.contains("statements in progress") && !t_class.starts_with("Busy") {
            return Divergence { verdict: Verdict::Unspecified, signature: format!("sqlite counts a statement that got SQLITE_BUSY as in progress: {pre_sig}"), detail };
        }
        if t_class == "Schema" && !s_is_err && self.s.conns() == 2 && self.saw_rollback_to {
            return Divergence { verdict: Verdict::Known, signature: format!("known G (ROLLBACK TO, schema changed): {pre_sig}"), detail };
        }
        if t_class == "Schema" && s_class != "Schema" && self.s.conns() == 2 {
            return Divergence { verdict: Verdict::Known, signature: format!("known G-family (re-prepare in a transaction after another connection's DDL): {pre_sig}"), detail };
        }
        if s_class == "Error" && (d.s.contains("already exists") || d.s.contains("duplicate column")) && t_class == "Busy" {
            return Divergence { verdict: Verdict::Unspecified, signature: format!("error precedence: SQLite's schema error before Turso's BUSY: {pre_sig}"), detail };
        }
        let vt_sql = match label.split_once(": ") {
            Some((h, q)) if h.starts_with("exec") => q.to_string(),
            _ => self.last_slot.and_then(|k| self.meta.get(&k)).map_or(String::new(), |m| m.sql.clone()),
        };
        if t_class == "Busy" && s_class != "Busy" && label.starts_with("step ") && vt_sql.contains("aux.") && self.last_slot.is_some_and(|k| self.turso_busy_slots.contains(&k)) {
            return Divergence { verdict: Verdict::Known, signature: format!("known aux-busy-retry (retry after Busy on the attached database stays Busy): {pre_sig}"), detail };
        }
        if s_class == "Busy" && is_vt_write(&vt_sql) {
            return Divergence { verdict: Verdict::Unspecified, signature: format!("SQLite takes main's write lock for a virtual-table write: {pre_sig}"), detail };
        }
        if d.rows && int_real_only(&d.s, &d.t) {
            return Divergence { verdict: Verdict::Unspecified, signature: format!("group key representative (INTEGER vs whole REAL): {pre_sig}"), detail };
        }
        if d.t.contains("Could not find Virtual Table to Destroy") && self.known.contains(&"VTREG") {
            return Divergence { verdict: Verdict::Known, signature: format!("known vt-registry (DROP after RENAME or DROP+ROLLBACK): {pre_sig}"), detail };
        }
        if d.t.contains("Could not find Virtual Table to Destroy") && self.s.conns() == 2 {
            return Divergence { verdict: Verdict::Known, signature: format!("known vt-other-connection (DROP): {pre_sig}"), detail };
        }
        if t_class == "Schema" && !s_is_err && self.s.conns() == 2 && self.attach {
            return Divergence { verdict: Verdict::Known, signature: format!("known aux-schema-changed (reader transaction, attached schema changed): {pre_sig}"), detail };
        }
        let cur_sql = match label.split_once(": ") {
            Some((h, q)) if h.starts_with("exec") => q.to_string(),
            _ => self.last_slot.and_then(|k| self.meta.get(&k)).map_or(String::new(), |m| m.sql.clone()),
        };
        let ddl = ["CREATE", "DROP", "ALTER"].iter().any(|k| cur_sql.trim_start().to_uppercase().starts_with(k));
        if t_class == "Busy" && !s_is_err && self.s.conns() == 2 && self.attach && ddl && cur_sql.contains("aux.") {
            return Divergence { verdict: Verdict::Known, signature: format!("known aux-ddl-stale (DDL on the attached database needs main's snapshot): {pre_sig}"), detail };
        }
        if d.what.contains("last_insert_rowid()") && self.log.iter().rev().take(3).any(|l| l.contains("Constraint:")) {
            return Divergence { verdict: Verdict::Unspecified, signature: format!("last_insert_rowid() after a failed INSERT: {pre_sig}"), detail };
        }
        if d.what.contains("total_changes()") && self.vt_trigger {
            return Divergence { verdict: Verdict::Known, signature: format!("known vt-changes (trigger writes a virtual table): {pre_sig}"), detail };
        }
        if d.what.contains("total_changes()") {
            return Divergence { verdict: Verdict::Known, signature: format!("known K (total_changes): {pre_sig}"), detail };
        }
        if self.sqlite_busy_taint || self.busy_pending.iter().any(|k| self.meta.contains_key(k)) {
            return Divergence { verdict: Verdict::Unspecified, signature: format!("sqlite keeps the read transaction of an unreset statement that got SQLITE_BUSY: {pre_sig}"), detail };
        }
        if t_class == "Busy" && s_class != "Busy" && self.s.conns() == 2 && self.aux_ddl_txn.iter().any(|b| *b) {
            return Divergence { verdict: Verdict::Known, signature: format!("known aux-ddl-stale (DDL on the attached database holds main): {pre_sig}"), detail };
        }
        if t_class == "Busy" && s_class != "Busy" && (quiet_ddl || (self.s.conns() == 2 && self.quiet_ddl_txn.iter().any(|b| *b))) {
            return Divergence { verdict: Verdict::Known, signature: format!("known F (no-op DDL takes the write lock): {pre_sig}"), detail };
        }
        if s_class == "Locked" && kind.contains(" DROP") {
            return Divergence { verdict: Verdict::Unspecified, signature: format!("sqlite LOCKED: DROP with an active statement (Turso allows; #7728 area): {pre_sig}"), detail };
        }
        if self.s.conns() == 2 && self.meta.values().any(|m| m.suspended && { let t = tables_of(&m.sql); !t.is_empty() && t.iter().all(|x| x.starts_with('u')) }) {
            return Divergence { verdict: Verdict::Known, signature: format!("known #10825 (aux-only statement starts main's snapshot): {pre_sig}"), detail };
        }
        if self.vt_multirow {
            return Divergence { verdict: Verdict::Known, signature: format!("known vt-multirow (multi-row INSERT into a virtual table): {pre_sig}"), detail };
        }
        if self.s.conns() == 2 && self.aux_first.iter().any(|b| *b) {
            return Divergence { verdict: Verdict::Known, signature: format!("known #10825 (aux-only statement starts main's snapshot): {pre_sig}"), detail };
        }
        if self.s.conns() == 2 && self.sp_begin.iter().any(|b| *b) {
            return Divergence { verdict: Verdict::Known, signature: format!("known #11394 (SAVEPOINT snapshot): {pre_sig}"), detail };
        }
        for k in ["B", "E", "A", "9323"] {
            if self.known.contains(&k) {
                return Divergence { verdict: Verdict::Known, signature: format!("known {k}: {pre_sig}"), detail };
            }
        }
        if self.known.contains(&"VTREG") {
            return Divergence { verdict: Verdict::Known, signature: format!("known vt-registry (after a vtab RENAME or DROP inside a transaction): {pre_sig}"), detail };
        }
        if self.known.contains(&"VTROLLBACK") {
            return Divergence { verdict: Verdict::Known, signature: format!("known vt-rollback (failed statement rolls back the virtual table): {pre_sig}"), detail };
        }
        if !self.mvcc && self.s.conns() == 2 && t_err.contains("snapshot is stale") && !s_is_err {
            let other_update = self.log.iter().any(|l| (l.starts_with("exec c") || l.starts_with("prepare s")) && (l.contains(": UPDATE") || l.contains(" DO UPDATE ")));
            if other_update {
                return Divergence { verdict: Verdict::Unspecified, signature: format!("possibly SQLite's identical-content overwrite (an UPDATE that rewrites equal bytes commits no WAL frame; Turso's makes other snapshots stale): {pre_sig_early}"), detail };
            }
        }
        let overlapped_any = self.last_slot.and_then(|k| self.meta.get(&k)).is_some_and(|m| m.overlapped);
        if overlapped_any && s_class == "Corrupt" {
            return Divergence { verdict: Verdict::Unspecified, signature: format!("overlap: sqlite CORRUPT: {pre_sig}"), detail };
        }
        let overlapped = label.starts_with("step ")
            && self.last_slot.and_then(|k| self.meta.get(&k)).is_some_and(|m| m.overlapped)
            && (d.rows || (row_or_done(&d.s) && row_or_done(&d.t)));
        let sig_core = if d.rows {
            format!("{kind}: rows differ")
        } else if d.what.starts_with("state") {
            let w = d.what.split_whitespace().nth(1).unwrap_or("");
            format!("{kind}: state {w} differs")
        } else {
            let k = |x: &str| x.split(':').next().unwrap_or("").split_whitespace().next().unwrap_or("").to_string();
            format!("{kind}: sqlite {} / turso {}", k(&d.s), k(&d.t))
        };
        if overlapped {
            return Divergence { verdict: Verdict::Unspecified, signature: format!("overlap: {sig_core}"), detail };
        }
        Divergence { verdict: Verdict::Finding, signature: sig_core, detail }
    }
}

fn dump_queries() -> &'static str {
    "SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '__turso_internal_%' AND sql NOT LIKE 'CREATE VIRTUAL%' ORDER BY name"
}

fn dump_queries_aux() -> &'static str {
    "SELECT name FROM aux.sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '__turso_internal_%' ORDER BY name"
}

fn rusqlite_dump(path: &Path) -> Result<Vec<String>, String> {
    let db = rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE).map_err(|e| e.to_string())?;
    // SAFETY: the handle is open; the module and functions outlive no one.
    unsafe {
        engines::register_sqlite(db.handle());
        vt::register_sqlite(db.handle(), &Arc::default());
    }
    let mut out = Vec::new();
    let ic: String = db.query_row("PRAGMA integrity_check", [], |r| r.get(0)).map_err(|e| format!("integrity_check: {e}"))?;
    out.push(format!("integrity_check: {ic}"));
    let names: Vec<String> = {
        let mut st = db.prepare(dump_queries()).map_err(|e| e.to_string())?;
        let v = st.query_map([], |r| r.get(0)).map_err(|e| e.to_string())?.collect::<Result<_, _>>().map_err(|e| e.to_string())?;
        v
    };
    for n in names {
        out.push(format!("table {n}"));
        let mut st = db.prepare(&format!("SELECT * FROM \"{n}\" ORDER BY rowid")).map_err(|e| e.to_string())?;
        let cols = st.column_count();
        let mut rows = st.query([]).map_err(|e| e.to_string())?;
        while let Some(r) = rows.next().map_err(|e| e.to_string())? {
            let cells: Vec<Cell> = (0..cols)
                .map(|k| match r.get_ref(k).unwrap() {
                    rusqlite::types::ValueRef::Null => Cell::Null,
                    rusqlite::types::ValueRef::Integer(i) => Cell::Int(i),
                    rusqlite::types::ValueRef::Real(x) => Cell::Real(x),
                    rusqlite::types::ValueRef::Text(t) => Cell::Text(String::from_utf8_lossy(t).into_owned()),
                    rusqlite::types::ValueRef::Blob(b) => Cell::Blob(b.to_vec()),
                })
                .collect();
            out.push(format!("  {}", show_cells(&cells)));
        }
    }
    Ok(out)
}

struct Outcome {
    div: Option<Divergence>,
    log: Vec<String>,
}

fn run_session(sess: &Session, dir: &Path, tag: &str, stats: &mut Counters, verbose: bool) -> Outcome {
    let sp = dir.join(format!("s-{tag}.db"));
    let tp = dir.join(format!("t-{tag}.db"));
    let sa = dir.join(format!("s-{tag}-aux.db"));
    let ta = dir.join(format!("t-{tag}-aux.db"));
    let sa2 = dir.join(format!("s-{tag}-aux2.db"));
    let ta2 = dir.join(format!("t-{tag}-aux2.db"));
    for p in [&sp, &tp, &sa, &ta, &sa2, &ta2] {
        for ext in ["", "-wal", "-shm", "-journal", "-log"] {
            let _ = fs::remove_file(format!("{}{ext}", p.display()));
        }
    }
    let mentions = |needle: &str| sess.setup.iter().any(|q| q.contains(needle)) || sess.actions.iter().any(|a| matches!(a, Action::Prepare { sql, .. } | Action::Exec { sql, .. } if sql.contains(needle)));
    let attach = mentions("ATTACH");
    let attach2 = mentions(engines::AUX2);
    if attach {
        // The attached files start as empty WAL databases written by SQLite.
        let files: Vec<&PathBuf> = if attach2 { vec![&sa, &ta, &sa2, &ta2] } else { vec![&sa, &ta] };
        for p in files {
            if let Ok(db) = rusqlite::Connection::open(p) {
                let _ = db.query_row("PRAGMA journal_mode=WAL", [], |r| r.get::<_, String>(0));
            }
        }
    }
    let s = match Sqlite::open(&sp.to_string_lossy(), sess.conns, &sa.to_string_lossy(), &sa2.to_string_lossy()) {
        Ok(s) => s,
        Err(e) => return Outcome { div: Some(Divergence { verdict: Verdict::Skip, signature: format!("harness: {e}"), detail: e }), log: vec![] },
    };
    let t = guarded(|| Turso::open(&tp.to_string_lossy(), sess.conns, &ta.to_string_lossy(), &ta2.to_string_lossy(), sess.mvcc), |m| Err(format!("PANIC {m}")));
    let t = match t {
        Ok(t) => t,
        Err(e) => {
            let v = if e.starts_with("PANIC") { Verdict::Finding } else { Verdict::Skip };
            return Outcome { div: Some(Divergence { verdict: v, signature: format!("open: {}", normalize(&e)), detail: e }), log: vec![] };
        },
    };
    let cascades = sess.setup.iter().any(|q| q.contains("TRIGGER") || q.contains("REFERENCES"));
    let mut r = Runner { s, t: Some(t), meta: BTreeMap::new(), tainted: None, log: Vec::new(), stats, verbose, cascades, last_slot: None, index_changes: vec![false; sess.conns], sp_begin: vec![false; sess.conns], window_rowid: vec![false; sess.conns], known: Vec::new(), schema_epoch: 0, epochs: BTreeMap::new(), attach, vtabs: sess.setup.iter().filter_map(|q| q.strip_prefix("CREATE VIRTUAL TABLE ").and_then(|x| x.split_whitespace().next()).map(String::from)).collect(), saw_rollback_to: false, vt_total_offset: vec![0; sess.conns], vt_write_conn: None, vt_rowid: vec![false; sess.conns], txn_touched: vec![false; sess.conns], aux_first: vec![false; sess.conns], vt_multirow: false, aux_touched: vec![false; sess.conns], quiet_ddl_txn: vec![false; sess.conns], aux_ddl_txn: vec![false; sess.conns], busy_pending: Default::default(), sqlite_busy_taint: false, turso_busy_slots: Default::default(), mvcc: sess.mvcc, mvcc_deferred: false, ckpt_any: false, ckpt_reset: false, concurrent_txn: vec![false; sess.conns], mvcc_seq_gap: false, temp: sess.setup.iter().any(|q| q.contains("TEMP TABLE")), vt_trigger: sess.setup.iter().any(|q| q.starts_with("CREATE TRIGGER trv") && q.contains("INSERT INTO vt")) };
    let mut div = None;
    // Setup: one-shot statements on connection 0.
    for q in &sess.setup {
        r.vt_write_conn = is_vt_write(q).then_some(0);
        let (sv, tv) = r.exec_both(0, q);
        r.log(format!("setup: {q}\n    sqlite: {}\n    turso:  {}", sv.iter().map(show_ev).collect::<Vec<_>>().join(", "), tv.iter().map(show_ev).collect::<Vec<_>>().join(", ")));
        r.log_vt_events();
        if resets_changes(&stmt_kind(q), q) || is_vt_write(q) {
            r.index_changes[0] = true;
        } else if q.starts_with("INSERT") {
            r.index_changes[0] = false;
        }
        if let Some(d) = r.compare_evs(None, &sv, &tv, false).or_else(|| r.compare_states()) {
            div = Some(r.classify(&d, &format!("setup {}", stmt_kind(q)), &format!("setup: {q}")));
            break;
        }
    }
    if div.is_none() {
        let keep_going = std::env::var_os("APIDIFF_CONTINUE").is_some();
        for a in &sess.actions {
            if let Err((d, kind)) = r.run_action(a) {
                let v = r.classify(&d, &kind, &gen::render_action(a));
                if keep_going {
                    r.log(format!("    !! {:?}: {}", v.verdict, v.signature));
                    div.get_or_insert(v);
                    continue;
                }
                div = Some(v);
                break;
            }
        }
    }
    if div.is_none() {
        div = final_checks(&mut r);
    }
    let log = std::mem::take(&mut r.log);
    let busy_taint = r.sqlite_busy_taint;
    let mvcc_commit = r.known.contains(&"MVCCCOMMIT");
    let mvcc_deferred = r.mvcc_deferred;
    let mvcc_seq_gap = r.mvcc_seq_gap;
    let file_known: Option<&'static str> = ["A", "B", "E", "9323", "VTROLLBACK", "VTREG", "AUXBUSY"].into_iter().find(|k| r.known.contains(k));
    // Close: Turso first (its drop may panic after a panic).
    if let Some(mut t) = r.t.take() {
        let _ = panic::catch_unwind(AssertUnwindSafe(move || {
            t.close();
            drop(t);
        }));
    }
    drop(r);
    if div.is_none() {
        // Both files read by SQLite (and the attached files).
        let both = |x: &Path, y: &Path, y2: &Path| -> Result<Vec<String>, String> {
            let mut v = rusqlite_dump(x)?;
            if attach {
                v.push("== aux".into());
                v.extend(rusqlite_dump(y)?);
            }
            if attach2 {
                v.push("== aux2".into());
                v.extend(rusqlite_dump(y2)?);
            }
            Ok(v)
        };
        let a = both(&sp, &sa, &sa2);
        let b = both(&tp, &ta, &ta2);
        let same = match (&a, &b) {
            (Ok(x), Ok(y)) => x == y,
            _ => false,
        };
        if !same {
            let show = |x: &Result<Vec<String>, String>| match x {
                Ok(v) => v.join("\n    "),
                Err(e) => format!("error: {e}"),
            };
            let sig = match &b {
                Err(e) => format!("file: SQLite cannot read Turso's file: {}", normalize(e)),
                Ok(v) if !v[0].ends_with(": ok") => "file: integrity_check of Turso's file fails in SQLite".into(),
                _ => "file: contents differ after close".into(),
            };
            let mvcc_index_9285 = sess.mvcc && matches!(&b, Ok(v) if v[0].contains("values differ from index"));
            let (verdict, sig) = if mvcc_index_9285 {
                (Verdict::Known, format!("known #9285 (MVCC: index keeps the old key after an update to an equal-comparing value): {sig}"))
            } else if busy_taint {
                (Verdict::Unspecified, format!("sqlite keeps the transaction of an unreset statement that got SQLITE_BUSY: {sig}"))
            } else if let Some(k) = file_known.filter(|_| !mvcc_commit) {
                (Verdict::Known, format!("known {k}: {sig}"))
            } else if mvcc_commit {
                (Verdict::Known, format!("known mvcc-commit-suspended: {sig}"))
            } else if mvcc_seq_gap {
                (Verdict::Skip, "MVCC: AUTOINCREMENT value kept after a rollback or failed statement in BEGIN CONCURRENT (inner sequence transaction; gaps permitted by autoinc.html)".into())
            } else if mvcc_deferred {
                (Verdict::Skip, "documented: MVCC shared transaction (write committed only when the last sibling statement finishes)".into())
            } else {
                (Verdict::Finding, sig)
            };
            div = Some(Divergence { verdict, signature: sig, detail: format!("SQLite's file:\n    {}\nTurso's file (read by SQLite):\n    {}", show(&a), show(&b)) });
        }
    }
    for p in [&sp, &tp, &sa, &ta, &sa2, &ta2] {
        for ext in ["", "-wal", "-shm", "-journal", "-log"] {
            let _ = fs::remove_file(format!("{}{ext}", p.display()));
        }
    }
    Outcome { div, log }
}

fn final_checks(r: &mut Runner) -> Option<Divergence> {
    let slots: Vec<usize> = r.meta.keys().copied().collect();
    for slot in slots {
        if let Some(d) = r.end_statement(slot) {
            return Some(r.classify(&d, "final RETURNING", "(end of session)"));
        }
        r.abandon_check(slot);
        r.note_end(slot);
        r.s.finalize(slot);
        let tt = r.turso();
        if let Err(m) = guarded(
            || {
                tt.finalize(slot);
                Ok(())
            },
            Err,
        ) {
            return Some(Divergence { verdict: Verdict::Finding, signature: format!("panic: {}", normalize(&m)), detail: format!("finalizing s{slot}: {m}") });
        }
        let k = r.meta.get(&slot).map_or(String::new(), |m| stmt_kind(&m.sql));
        if let Some(d) = r.compare_states() {
            return Some(r.classify(&d, &format!("finalize {k}"), &format!("(end of session) finalize s{slot}")));
        }
    }
    r.meta.clear();
    let mut checks: Vec<String> = vec![dump_queries().into()];
    let (sv, tv) = r.exec_both(0, dump_queries());
    if let Some(d) = r.compare_evs(None, &sv, &tv, false) {
        return Some(r.classify(&d, "final table list", "(end of session)"));
    }
    for e in &sv {
        if let Ev::Row(row) = e {
            if let Some(Cell::Text(n)) = row.first() {
                checks.push(format!("SELECT * FROM \"{n}\" ORDER BY rowid"));
            }
        }
    }
    // Round 3: the attached databases present at the end (DETACH/ATTACH
    // run as actions), by name; then their tables.
    let mut dbs: Vec<String> = Vec::new();
    if r.attach {
        let (sv, tv) = r.exec_both(0, "PRAGMA database_list");
        let names = |v: &[Ev]| -> Vec<Ev> {
            let mut n: Vec<String> = v.iter().filter_map(|e| if let Ev::Row(row) = e { row.get(1).and_then(|c| if let Cell::Text(t) = c { Some(t.clone()) } else { None }) } else { None }).filter(|n| n != "main" && n != "temp").collect();
            n.sort();
            n.into_iter().map(|x| Ev::Row(vec![Cell::Text(x)])).collect()
        };
        let (ns, nt) = (names(&sv), names(&tv));
        if let Some(d) = r.compare_evs(None, &ns, &nt, false) {
            return Some(r.classify(&d, "final database list", "(end of session) PRAGMA database_list"));
        }
        for e in &ns {
            if let Ev::Row(row) = e {
                if let Some(Cell::Text(n)) = row.first() {
                    dbs.push(n.clone());
                }
            }
        }
    }
    let temp = r.temp;
    if temp {
        dbs.push("temp".into());
    }
    for db in dbs {
        let q = dump_queries_aux().replace("aux.", &format!("{db}."));
        let (sv, tv) = r.exec_both(0, &q);
        if let Some(d) = r.compare_evs(None, &sv, &tv, false) {
            return Some(r.classify(&d, "final aux table list", &format!("(end of session) {q}")));
        }
        for e in &sv {
            if let Ev::Row(row) = e {
                if let Some(Cell::Text(n)) = row.first() {
                    checks.push(format!("SELECT * FROM {db}.\"{n}\" ORDER BY rowid"));
                }
            }
        }
    }
    // Virtual tables present at the end (RENAME/DROP run as actions).
    let (sv, tv) = r.exec_both(0, "SELECT name FROM sqlite_schema WHERE type = 'table' AND sql LIKE 'CREATE VIRTUAL%' ORDER BY name");
    if let Some(d) = r.compare_evs(None, &sv, &tv, false) {
        return Some(r.classify(&d, "final vtab list", "(end of session)"));
    }
    for e in &sv {
        if let Ev::Row(row) = e {
            if let Some(Cell::Text(n)) = row.first() {
                checks.push(format!("SELECT rowid, a, b FROM \"{n}\" ORDER BY rowid"));
            }
        }
    }
    checks.push("PRAGMA integrity_check".into());
    for q in &checks[1..] {
        let (sv, tv) = r.exec_both(0, q);
        r.log(format!("final: {q}\n    sqlite: {}\n    turso:  {}", sv.iter().map(show_ev).collect::<Vec<_>>().join(", "), tv.iter().map(show_ev).collect::<Vec<_>>().join(", ")));
        if let Some(d) = r.compare_evs(None, &sv, &tv, false) {
            let kind = if q.starts_with("PRAGMA") { "final integrity_check" } else { "final table dump" };
            return Some(r.classify(&d, kind, &format!("final: {q}")));
        }
    }
    for c in 0..r.s.conns() {
        if !r.s.state(c).autocommit {
            let (sv, tv) = r.exec_both(c, "COMMIT");
            r.log(format!("final: c{c} COMMIT\n    sqlite: {}\n    turso:  {}", sv.iter().map(show_ev).collect::<Vec<_>>().join(", "), tv.iter().map(show_ev).collect::<Vec<_>>().join(", ")));
            if let Some(d) = r.compare_evs(None, &sv, &tv, false).or_else(|| r.compare_states()) {
                return Some(r.classify(&d, "final COMMIT", "final: COMMIT"));
            }
        }
    }
    // Leave nothing in Turso's WAL for SQLite to recover.
    let attach = r.attach;
    let mvcc = r.mvcc;
    let tt = r.turso();
    let mut qs = vec!["PRAGMA wal_checkpoint(TRUNCATE)"];
    if mvcc {
        // An MVCC database is not readable by SQLite (header); back to WAL,
        // which checkpoints the logical log first (manual.md).
        qs.push("PRAGMA journal_mode = wal");
    }
    if attach {
        qs.extend(["PRAGMA aux.wal_checkpoint(TRUNCATE)", "PRAGMA aux2.wal_checkpoint(TRUNCATE)"]);
    }
    let out = guarded(
        || {
            let mut out = Vec::new();
            for q in qs {
                let mut v = vec![tt.prepare(HIDDEN, 0, q)];
                if v[0] == Ev::Ok {
                    loop {
                        let e = tt.step(HIDDEN);
                        let stop = !matches!(e, Ev::Row(_));
                        v.push(e);
                        if stop {
                            break;
                        }
                    }
                }
                tt.finalize(HIDDEN);
                out.push(format!("final (turso only): {q}\n    turso:  {}", v.iter().map(show_ev).collect::<Vec<_>>().join(", ")));
            }
            out
        },
        |m| vec![format!("final (turso only): PANIC {m}")],
    );
    for l in out {
        r.log(l);
    }
    None
}

// ------------------------------------------------------------ shrinking

fn reproduces(sess: &Session, dir: &Path, sig: &str, n: &mut u64) -> bool {
    *n += 1;
    let mut c = Counters::default();
    let o = run_session(sess, dir, &format!("shrink-{}-{n}", sess.seed), &mut c, false);
    o.div.is_some_and(|d| d.signature == sig)
}

fn shrink(sess: &Session, dir: &Path, sig: &str, budget: Duration) -> (Session, u64) {
    let start = Instant::now();
    let mut best = sess.clone();
    let mut tries = 0u64;
    let mut progress = true;
    while progress && start.elapsed() < budget {
        progress = false;
        for chunk in [8usize, 4, 2, 1] {
            let mut i = best.actions.len();
            while i > 0 && start.elapsed() < budget {
                let lo = i.saturating_sub(chunk);
                let mut cand = best.clone();
                cand.actions.drain(lo..i);
                if reproduces(&cand, dir, sig, &mut tries) {
                    best = cand;
                    progress = true;
                }
                i = lo;
            }
        }
        let mut i = best.setup.len();
        while i > 0 && start.elapsed() < budget {
            i -= 1;
            let mut cand = best.clone();
            cand.setup.remove(i);
            if reproduces(&cand, dir, sig, &mut tries) {
                best = cand;
                progress = true;
            }
        }
        // Fewer steps, and one connection.
        for k in 0..best.actions.len() {
            if let Action::Step { slot, n } = best.actions[k] {
                for m in [1, 2, 3] {
                    if m < n {
                        let mut cand = best.clone();
                        cand.actions[k] = Action::Step { slot, n: m };
                        if reproduces(&cand, dir, sig, &mut tries) {
                            best = cand;
                            progress = true;
                            break;
                        }
                    }
                }
            }
        }
        if best.conns == 2 && best.actions.iter().all(|a| !matches!(a, Action::Prepare { conn: 1, .. } | Action::Exec { conn: 1, .. })) {
            let mut cand = best.clone();
            cand.conns = 1;
            if reproduces(&cand, dir, sig, &mut tries) {
                best = cand;
                progress = true;
            }
        }
    }
    (best, tries)
}

// ------------------------------------------------------------ runner

fn slug(s: &str) -> String {
    let mut out: String = s.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    while out.contains("--") {
        out = out.replace("--", "-");
    }
    out.trim_matches('-').chars().take(80).collect()
}

struct Shared {
    stats: Mutex<Counters>,
    kept: Mutex<BTreeMap<String, u64>>,
    out: PathBuf,
    stop: AtomicBool,
    sessions: AtomicU64,
    current: Mutex<Vec<Option<(u64, Instant)>>>,
}

fn record(sh: &Shared, sess: &Session, div: &Divergence, dir: &Path, log: &[String]) {
    let bucket = match div.verdict {
        Verdict::Finding => "findings",
        Verdict::Unspecified => "unspecified",
        Verdict::Skip => "skips",
        Verdict::Known => "known",
        Verdict::Clean => return,
    };
    let n = {
        let mut k = sh.kept.lock().unwrap();
        let e = k.entry(format!("{bucket}/{}", div.signature)).or_default();
        *e += 1;
        *e
    };
    let limit = if div.verdict == Verdict::Skip { 1 } else { EXAMPLES as u64 };
    if n > limit {
        return;
    }
    let d = sh.out.join(bucket).join(slug(&div.signature));
    let _ = fs::create_dir_all(&d);
    let mut text = format!("signature: {}\nseed: {}\n\n{}\n\n== session\n{}\n", div.signature, sess.seed, div.detail, gen::render(sess));
    if n == 1 && div.verdict == Verdict::Finding && std::env::var_os("APIDIFF_NOSHRINK").is_none() {
        let (small, tries) = shrink(sess, dir, &div.signature, Duration::from_secs(90));
        let _ = write!(text, "\n== shrunk ({tries} runs)\n{}\n", gen::render(&small));
        let mut c = Counters::default();
        let o = run_session(&small, dir, &format!("shrunk-{}", sess.seed), &mut c, false);
        let _ = write!(text, "\n== shrunk trace\n{}\n", o.log.join("\n"));
        let _ = fs::write(d.join(format!("seed-{}.shrunk.txt", sess.seed)), gen::render(&small));
    } else {
        let _ = write!(text, "\n== trace\n{}\n", log.join("\n"));
    }
    let _ = fs::write(d.join(format!("seed-{}.txt", sess.seed)), text);
}

fn worker(sh: Arc<Shared>, w: usize, jobs: usize, seed0: u64) {
    let dir = sh.out.join("tmp").join(format!("w{w}"));
    let _ = fs::create_dir_all(&dir);
    let mut k = 0u64;
    while !sh.stop.load(Ordering::Relaxed) {
        let seed = seed0 + w as u64 + k * jobs as u64;
        k += 1;
        sh.current.lock().unwrap()[w] = Some((seed, Instant::now()));
        let sess = gen::session(seed, GEN.load(Ordering::Relaxed));
        let mut c = Counters::default();
        c.add("sessions", 1);
        if sess.conns == 2 {
            c.add("sessions with 2 connections", 1);
        }
        let o = run_session(&sess, &dir, &seed.to_string(), &mut c, false);
        match &o.div {
            None => c.add("sessions clean", 1),
            Some(d) => {
                let b = match d.verdict {
                    Verdict::Finding => "finding",
                    Verdict::Unspecified => "unspecified",
                    Verdict::Skip => "skip",
                    Verdict::Known => "known",
                    Verdict::Clean => "clean",
                };
                c.add(format!("{b}: {}", d.signature), 1);
                c.add(format!("sessions ended by {b}"), 1);
                record(&sh, &sess, d, &dir, &o.log);
            },
        }
        sh.stats.lock().unwrap().merge(&c);
        sh.sessions.fetch_add(1, Ordering::Relaxed);
        sh.current.lock().unwrap()[w] = None;
    }
}

fn report(c: &Counters, elapsed: Duration) -> String {
    let mut s = format!("elapsed {:.0}s\n", elapsed.as_secs_f64());
    for (k, v) in &c.0 {
        let _ = writeln!(s, "{v:>10}  {k}");
    }
    s
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn trace(sess: &Session) {
    println!("{}", gen::render(sess));
    let dir = std::env::temp_dir().join(format!("apidiff-one-{}", std::process::id()));
    let _ = fs::create_dir_all(&dir);
    let mut c = Counters::default();
    let o = run_session(sess, &dir, "one", &mut c, true);
    match o.div {
        None => println!("== clean"),
        Some(d) => println!("== {:?}: {}\n{}", d.verdict, d.signature, d.detail),
    }
    let _ = fs::remove_dir_all(&dir);
}

fn main() {
    panic::set_hook(Box::new(|info| {
        let at = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        PANIC_AT.with(|a| *a.borrow_mut() = at);
        let m = info.payload().downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| info.payload().downcast_ref::<String>().cloned()).unwrap_or_default();
        let at2 = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        if !(at2.contains("turso-main") || at2.contains("/turso/")) {
            eprintln!("harness panic: {m} at {at2}");
        }
    }));
    let args: Vec<String> = std::env::args().collect();
    if let Some(g) = arg(&args, "--gen") {
        GEN.store(g.parse().expect("--gen 1|2"), Ordering::Relaxed);
    }
    match args.get(1).map(String::as_str) {
        Some("one") => {
            let seed: u64 = args[2].parse().expect("seed");
            trace(&gen::session(seed, GEN.load(Ordering::Relaxed)));
        },
        Some("replay") => {
            let s = gen::parse(&fs::read_to_string(&args[2]).expect("file")).expect("parse");
            trace(&s);
        },
        Some("shrink") => {
            let s = gen::parse(&fs::read_to_string(&args[2]).expect("file")).expect("parse");
            let dir = std::env::temp_dir().join(format!("apidiff-shrink-{}", std::process::id()));
            let _ = fs::create_dir_all(&dir);
            let mut c = Counters::default();
            let o = run_session(&s, &dir, "orig", &mut c, false);
            let Some(d) = o.div else {
                println!("clean: nothing to shrink");
                return;
            };
            let sig = arg(&args, "--signature").unwrap_or(d.signature);
            let (small, tries) = shrink(&s, &dir, &sig, Duration::from_secs(300));
            println!("-- {sig} ({tries} runs)\n{}", gen::render(&small));
        },
        Some("run") => {
            let jobs: usize = arg(&args, "--jobs").map_or(4, |x| x.parse().unwrap());
            let minutes: f64 = arg(&args, "--minutes").map_or(10.0, |x| x.parse().unwrap());
            let seed: u64 = arg(&args, "--seed").map_or(1, |x| x.parse().unwrap());
            let out = PathBuf::from(arg(&args, "--out").unwrap_or_else(|| format!("runs/{seed}")));
            let _ = fs::create_dir_all(&out);
            let sh = Arc::new(Shared {
                stats: Mutex::new(Counters::default()),
                kept: Mutex::new(BTreeMap::new()),
                out: out.clone(),
                stop: AtomicBool::new(false),
                sessions: AtomicU64::new(0),
                current: Mutex::new(vec![None; jobs]),
            });
            let start = Instant::now();
            let handles: Vec<_> = (0..jobs)
                .map(|w| {
                    let sh = sh.clone();
                    thread::Builder::new().stack_size(64 << 20).spawn(move || worker(sh, w, jobs, seed)).unwrap()
                })
                .collect();
            let deadline = Duration::from_secs_f64(minutes * 60.0);
            let mut hung = Vec::new();
            let mut last = Instant::now();
            while start.elapsed() < deadline {
                thread::sleep(Duration::from_millis(500));
                for (w, cur) in sh.current.lock().unwrap().iter().enumerate() {
                    if let Some((seed, t)) = cur {
                        if t.elapsed() > HANG && !hung.contains(&w) {
                            hung.push(w);
                            let d = out.join("findings").join("hang");
                            let _ = fs::create_dir_all(&d);
                            let _ = fs::write(d.join(format!("seed-{seed}.txt")), gen::render(&gen::session(*seed, GEN.load(Ordering::Relaxed))));
                            eprintln!("worker {w} hung on seed {seed}");
                        }
                    }
                }
                if last.elapsed() > Duration::from_secs(60) {
                    last = Instant::now();
                    eprintln!("{:.0}s: {} sessions", start.elapsed().as_secs_f64(), sh.sessions.load(Ordering::Relaxed));
                }
            }
            sh.stop.store(true, Ordering::Relaxed);
            for (w, h) in handles.into_iter().enumerate() {
                if !hung.contains(&w) {
                    let _ = h.join();
                }
            }
            let mut text = report(&sh.stats.lock().unwrap(), start.elapsed());
            let _ = writeln!(text, "hung workers: {hung:?}");
            let _ = fs::write(out.join("stats.txt"), &text);
            println!("{text}");
            let _ = fs::remove_dir_all(out.join("tmp"));
            std::process::exit(0);
        },
        _ => eprintln!("usage: turso-apidiff run|one SEED|replay FILE|shrink FILE"),
    }
}
