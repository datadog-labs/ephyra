//! The PostgreSQL fuzz runner: a supervisor, its worker processes and one
//! PostgreSQL cluster they share, for unattended runs of hours or days. The
//! port of `../sqlite-coverage`'s runner (same records, same layout, metrics
//! named `ephyra.postgresql.*` where SQLite's are `ephyra.sqlite.*`);
//! `RUNNER.md` in this crate is the operator's guide.
//!
//! - `runner run --dir D [--workers N] [--hours H] [--seed S] [--campaign
//!   C] [--tag T] [--statsd HOST:PORT] [--tags k:v,..] [--max-rss-mb MB]
//!   [--stale-secs T] [--pg-bin DIR] [--pg-dir DIR] [--fixed-bugs ID,..]`:
//!   the supervisor (`--fixed-bugs`: the known bugs the target has fixed,
//!   whose generator exclusions are lifted, `ephyra_postgresql::fixed`;
//!   kept in each input's `state.txt` as `fixed=`)
//!   ([`supervisor`]). It creates and starts the run's cluster (`D/pg`
//!   unless `--pg-dir`; binaries from `--pg-bin`, default
//!   `~/Projects/postgres/inst/bin`), starts the workers, restarts any that
//!   exit, kills and restarts hung ones, watches the memory of workers and
//!   of the cluster's backends (killing runaways), watches the server log
//!   for crashes (an assertion failure, a signal, a PANIC) and records them
//!   with every worker's input in flight, waits for crash recovery, and
//!   every minute sums the workers' totals into `D/stats.tsv` and
//!   `D/snapshots.jsonl` and DogStatsD metrics. It stops at the deadline or
//!   when `D/stop` exists, and stops the cluster then.
//! - `runner worker --dir D --id K --seed S --until T`: one worker
//!   ([`worker`]), started by the supervisor: one session (one database
//!   `e<pid>_0`) on the cluster, fresh `case_queries` inputs under the model
//!   property and the nine oracles in turn.
//! - `runner replay --finding FILE|DIR --out DIR [--dir D | --pg-dir DIR]
//!   [--pg-bin DIR] [--oracle NAME]`: an input rebuilt from a finding's
//!   `state.txt` (or a crash, hang, memory or lost record), written as a psql
//!   script and its Debug, and run once more ([`worker::replay`]).
//! - `runner version`: the version of the sources on disk and the one the
//!   binary was built from; exits 1 when they differ (a stale binary).
//!
//! Layout of `D`: `run.json`, `events.jsonl`, `snapshots.jsonl` (the run's
//! records, [`records`]); `events.log`, `stats.tsv` (readable views);
//! `seed`, `until`, `supervisor.state` (what a restarted supervisor
//! resumes from); `pg/` (the cluster: `data/`, the socket, `server.log`);
//! `workers/<id>/` (`state`, the input in flight; `stats`, the worker's
//! totals and heartbeat; `lost`, the input in flight when its session was
//! lost; `pid`; `log`); and `findings/` ([`findings`]).

mod content;
mod findings;
mod model;
mod records;
mod supervisor;
mod worker;

use std::{
    fs::{self, OpenOptions},
    io::Write as _,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use ephyra_postgresql::{
    cases::Case,
    expr::Query,
    setup::Step,
    sqlancer::{self, Tally},
};

pub type Input = (Case, Vec<Query>);
pub type Oracle = fn(&Case, &[Query]) -> Result<Tally, String>;

/// What a worker checks, in turn (input `n` under `ORACLES[n % 10]`): the
/// model property (`tests/cases.rs`, [`model::check`]) and the nine
/// oracles of `ephyra_postgresql::sqlancer`.
pub const ORACLES: [(&str, Oracle); 10] = [
    ("model", model::check),
    ("norec", sqlancer::norec),
    ("tlp", sqlancer::tlp),
    ("plans", sqlancer::plans),
    ("order", sqlancer::order),
    ("setops", sqlancer::setops),
    ("cte", sqlancer::ctes),
    ("window", sqlancer::windows),
    ("generic", sqlancer::generic),
    ("views", sqlancer::views),
];

/// Seconds since the epoch.
pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// The value after `flag` in `args`.
pub fn arg(args: &[String], flag: &str) -> Option<String> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1).cloned())
}

pub fn worker_dir(dir: &Path, id: usize) -> PathBuf {
    dir.join("workers").join(id.to_string())
}

/// Append a timestamped line to `D/events.log`.
pub fn event(dir: &Path, line: &str) {
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(dir.join("events.log")) {
        let _ = writeln!(f, "{} {line}", now());
    }
}

/// Write `contents` to `path` through a temporary file and a rename, so that
/// a reader never sees half of it.
pub fn write_atomic(path: &Path, contents: impl AsRef<[u8]>) {
    let tmp = path.with_extension("tmp");
    if fs::write(&tmp, contents).is_ok() {
        let _ = fs::rename(&tmp, path);
    }
}

/// `key=value` lines.
pub fn read_pairs(path: &Path) -> Vec<(String, String)> {
    pairs(&fs::read_to_string(path).unwrap_or_default())
}

pub fn pairs(text: &str) -> Vec<(String, String)> {
    text.lines().filter_map(|l| l.split_once('=')).map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

/// Seed mixing (splitmix64's finalizer), as sqlite-coverage's `mix`: input
/// `n` of worker `K` is drawn from `mix(mix(S, K), n)`.
pub fn mix(seed: u64, index: u64) -> u64 {
    let mut z = seed.wrapping_add(index.wrapping_mul(0x9e37_79b9_7f4a_7c15));
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// An input as a script psql runs in an empty database (`psql -X -v
/// ON_ERROR_STOP=0 -f script.sql`): the session settings the harness uses
/// (`pg::Session::fresh`), the extensions, the setup (COPY data inline),
/// the statements, a COMMIT when they leave a transaction open, and the
/// queries.
pub fn script(input: &Input, header: &str) -> String {
    let mut out = String::new();
    for l in header.lines() {
        out.push_str(&format!("-- {l}\n"));
    }
    out.push_str(
        "CREATE EXTENSION IF NOT EXISTS amcheck;\nCREATE EXTENSION IF NOT EXISTS btree_gin;\nCREATE EXTENSION IF NOT EXISTS btree_gist;\n\
         SET client_min_messages = warning;\nSET statement_timeout = '10s';\nDROP SCHEMA IF EXISTS s CASCADE;\nCREATE SCHEMA s;\nSET search_path = s, public;\n-- setup\n",
    );
    for step in input.0.setup.script() {
        match step {
            Step::Sql(sql) => out.push_str(&format!("{sql};\n")),
            Step::Copy(sql, data) => out.push_str(&format!("{sql};\n{data}\\.\n")),
        }
    }
    out.push_str("-- statements\n");
    for s in &input.0.statements {
        out.push_str(&format!("{s};\n"));
    }
    if !ephyra_postgresql::cases::final_model(&input.0).savepoints.is_empty() {
        out.push_str("COMMIT;\n");
    }
    out.push_str("-- queries\n");
    for q in &input.1 {
        out.push_str(&format!("{};\n", q.sql()));
    }
    out
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("run") => supervisor::run(&args),
        Some("worker") => worker::run(&args),
        Some("replay") => worker::replay(&args),
        Some("version") => {
            let (disk, built) = (records::version().1, records::built_version());
            println!("on disk {disk}, built from {built}");
            std::process::exit(i32::from(disk != built));
        },
        _ => {
            eprintln!("usage: runner run|worker|replay|version ...");
            std::process::exit(2);
        },
    }
}
