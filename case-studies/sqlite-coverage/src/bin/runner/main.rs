// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The fuzz runner: a supervisor and its worker processes, for unattended
//! runs of hours or days against the instrumented SQLite.
//!
//! - `runner run --dir D [--profiles NAME[+wal]:COUNT,..] [--fresh N]
//!   [--targeted M] [--hours H] [--seed S] [--campaign C] [--tag T]
//!   [--queries LO-HI] [--statsd HOST:PORT] [--tags k:v,..] [--max-rss-mb
//!   MB] [--stale-secs T] [--reference DRAWS] [--fixed-bugs BUG,..]`: the
//!   supervisor ([`supervisor`]; `--fixed-bugs`, the known bugs whose
//!   fixes the target SQLite has, goes into run.json's config). It gives each worker its loop ([`profiles`]: a
//!   profile, `gen`, `gen-deep`, `rare`, `novel`, `near-miss`, or `havoc`
//!   and `splice`, out of the default allocation; with `+wal`, its database
//!   in a WAL file in `$SQLITE_TMPDIR` or `/tmp` rather than in memory, and
//!   the storage oracle in its rotation; `--fresh` and `--targeted` count
//!   `gen` and `rare` workers when `--profiles` is not given), computes
//!   the `rare` workers' reference once (`D/reference.tsv`), starts the
//!   workers, restarts any that exit early, kills and restarts any whose
//!   heartbeat goes stale or whose memory passes the limit, records crashes
//!   and hangs with the input they were running, and every minute adds the
//!   workers' totals, overall and by profile, into `D/stats.tsv` and
//!   `D/snapshots.jsonl` and sends them as DogStatsD metrics. It stops at the deadline or when `D/stop`
//!   exists.
//! - `runner worker --dir D --id K --profile NAME[+wal] --seed S --until
//!   T`: one worker ([`worker`]), started by the supervisor (`--mode
//!   fresh|targeted` stands for `--profile gen|rare`).
//! - `runner reference --dir D [--draws N]`: the reference, started by the
//!   supervisor.
//! - `runner replay --state FILE --out DIR`: a fresh draw (of any profile)
//!   rebuilt from a crash or hang record, written as an example and run
//!   again, in a WAL file when the record's state says `storage=wal`.
//! - `runner version`: the version of the sources on disk and the one the
//!   binary was built from; exits 1 when they differ (a stale binary).
//!
//! Layout of `D`: `run.json`, `events.jsonl` and `snapshots.jsonl` (the
//! run's records, [`records`]); `events.log` and `stats.tsv` (readable views
//! of them); `reference.tsv`; `workers/<id>/` (`state`, the input in flight;
//! `stats`, the worker's totals and heartbeat; `reached`, the counters it
//! reached; `coverage-<start>.profraw`; `log`); and `findings/`
//! ([`findings`]). A WAL loop's database files are outside `D`, in
//! `$SQLITE_TMPDIR/ephyra-wal-<run>-w<id>/` (or `/tmp`'s), removed when the
//! worker stops.

mod content;
mod findings;
mod profiles;
mod records;
mod supervisor;
mod worker;

use std::{
    fs::{self, OpenOptions},
    io::Write as _,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use ephyra_sqlite::{
    cases::Case,
    expr::Query,
    sqlancer::{self, Tally},
};

pub type Input = (Case, Vec<Query>);
pub type Oracle = fn(&Case, &[Query]) -> Result<Tally, String>;

/// The oracles, run in turn: a memory loop's the first [`MEMORY_ORACLES`],
/// a WAL loop's all of them ([`profiles::Loop::oracles`]).
pub const ORACLES: [(&str, Oracle); 8] = [
    ("norec", sqlancer::norec),
    ("tlp", sqlancer::tlp),
    ("plans", sqlancer::plans),
    ("order", sqlancer::order),
    ("setops", sqlancer::setops),
    ("cte", sqlancer::ctes),
    ("window", sqlancer::windows),
    ("storage", sqlancer::storage),
];

/// The oracles of a memory loop, and the reference's: all but `storage`,
/// which compares memory with a WAL file.
pub const MEMORY_ORACLES: usize = 7;

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
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("run") => supervisor::run(&args),
        Some("worker") => worker::run(&args),
        Some("reference") => worker::reference(&args),
        Some("replay") => worker::replay(&args),
        Some("version") => {
            let (disk, built) = (records::version().1, records::built_version());
            println!("on disk {disk}, built from {built}");
            std::process::exit(i32::from(disk != built));
        },
        _ => {
            eprintln!("usage: runner run|worker|reference|replay|version ...");
            std::process::exit(2);
        },
    }
}
