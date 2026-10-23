// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! A worker: oracle inputs one after another until its deadline, the stop
//! file, or the supervisor's death (its parent process changes).
//!
//! Its loop is its profile's on its storage (`--profile NAME[+wal]`,
//! [`profiles::Loop`]; `--mode fresh|targeted` stands for `gen` and
//! `rare`). Input `n` of worker `K` is drawn from seed `mix(mix(S, K), n)`
//! at size `n % 101`, a fresh draw or a mutant of the profile's pool
//! ([`profiles::draw`]), and runs under oracle `n % 7` (`n % 8` in a WAL
//! loop, the eighth the storage oracle; or every oracle, by the profile),
//! with a 10-second deadline on its SQLite work. Before it runs,
//! `workers/K/state` names it (its profile, storage, whether it is a
//! mutant, and the oracle running); a restarted worker carries on from the
//! input after it, with its totals. A fresh draw reproduces from its seed;
//! a mutant came from the pool, which a restart empties. `rare`'s pool is
//! the targeted mode's of old, against `D/reference.tsv`.
//!
//! A WAL loop (`+wal`) sets ephyra-sqlite's storage to a WAL file before
//! each check (`sqlancer::Storage::Wal`), in its own directory
//! `ephyra-wal-<run>-w<K>` under `$SQLITE_TMPDIR`, else `/tmp` ([`wal_dir`];
//! best a tmpfs, since every input writes a database file, its WAL and its
//! WAL index): every oracle then runs on the file, and the storage oracle
//! compares it with memory. The directory is emptied when the worker starts
//! (of what a killed process left) and removed when it stops, after
//! `sqlancer::clear_storage`. Its metrics, snapshots, findings and the
//! state file say `storage` (`memory` or `wal`); `runner replay` of a WAL
//! loop's record runs in a WAL file too.
//!
//! With `--queries LO-HI` (or the profile's range), a fresh input's
//! database gets k queries in all, k drawn log-uniformly from LO to HI by
//! the input's seed ([`extend`]): one database is costly to build and serves
//! hundreds of queries, which cost less each the more share it. Deadlines
//! grow with the queries ([`deadline`]). A replay extends the same way (the
//! state file has the range).
//!
//! A failure is shrunk (candidates that fail with the same first line, each
//! under a deadline of 5 seconds and more with its queries, for at most 2
//! minutes) and recorded
//! ([`findings::record_failure`]); a statement error's example (`statement
//! N: ..`, no such column, table or index) keeps the input before shrinking
//! too, in `unshrunk/`, since a candidate with the same first line can fail
//! for another reason. Every 10 seconds, and during shrinking,
//! `workers/K/stats` is rewritten: the worker's totals and its heartbeat.
//! Every 30 seconds a `worker` line goes to `D/snapshots.jsonl`: the totals,
//! by oracle too (inputs, failures, timeouts, seconds), and the SQLite
//! counters reached so far (one input's in the profile's `sample`: every
//! input's but in `gen`, one in [`profiles::COVERAGE_SAMPLE`]), as a bitset
//! in `workers/K/reached` too (this process's, as the count is). Every
//! [`PROFILE_EVERY`], and when it stops, the process's whole coverage
//! profile (all its inputs' counters) goes to
//! `workers/K/coverage-<start>.profraw`, for `llvm-profdata` and `llvm-cov`
//! (`deploy/gaps.sh`).

use std::{
    collections::{BTreeMap, HashMap},
    net::UdpSocket,
    fs,
    ops::ControlFlow,
    panic::{self, AssertUnwindSafe},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use ephyra::{coverage, Gen, Source};
use ephyra_sqlite::{
    cases::case_metrics,
    expr::{ast_metrics, Queries},
    probes::{case_queries, final_schema},
    sqlancer::{self, Storage, Tally},
    FILE_FALLBACKS, QUERY_FALLBACKS,
};
use sqlite_coverage::{mix, warm_up};

use crate::{
    arg, findings, now,
    profiles::{self, Loop, Seeds, StorageKind},
    read_pairs,
    records::{self, esc, Identity},
    worker_dir, write_atomic, Input, MEMORY_ORACLES, ORACLES,
};

/// Draws in the reference (unless `--draws` says), and the seed they are
/// made from.
pub const REFERENCE: u64 = 30_000;
const REFERENCE_SEED: u64 = 0x005e_ed0f_0eef;

/// Timeouts saved per worker (unshrunk).
const SAVED_TIMEOUTS: u64 = 5;

/// With `--statsd`, a worker sends the time of one input in
/// [`LATENCY_SAMPLE`] and the shape of one in [`SHAPE_SAMPLE`] as DogStatsD
/// distributions (with their sample rates).
const LATENCY_SAMPLE: u64 = 10;
const SHAPE_SAMPLE: u64 = 50;

/// The shape metrics sent: of the queries ([`ast_metrics`]) and of the case
/// ([`case_metrics`]), each sent as `ephyra.sqlite.input.<name>`.
const SHAPE_QUERIES: &[&str] = &[
    "queries", "max depth", "nodes", "function calls", "aggregates", "joins", "subqueries", "subquery depth", "grouped queries",
    "compound queries", "ctes", "windows", "ordered queries",
];
const SHAPE_CASE: &[&str] = &["statements", "tables", "indexes", "rows", "selects", "inserts", "updates", "deletes"];

/// A DogStatsD client: the worker's distributions.
struct Statsd {
    socket: UdpSocket,
    addr: String,
    tags: String,
}

impl Statsd {
    fn distribution(&self, name: &str, value: f64, rate: f64, extra: &str) {
        let line = format!("ephyra.sqlite.{name}:{value}|d|@{rate}|#{}{extra}", self.tags);
        let _ = self.socket.send_to(line.as_bytes(), &self.addr);
    }
}

/// How often a worker writes its coverage profile.
const PROFILE_EVERY: Duration = Duration::from_secs(600);

/// Write the process's coverage profile to `path` (through a temporary file
/// and a rename).
fn save_profile(path: &Path) {
    if let Some(image) = coverage::profraw_image() {
        let tmp = path.with_extension("tmp");
        if fs::write(&tmp, image).is_ok() {
            let _ = fs::rename(&tmp, path);
        }
    }
}

/// One oracle's totals in a worker.
#[derive(Default, Clone, Copy)]
struct OracleTotals {
    execs: u64,
    failures: u64,
    timeouts: u64,
    ms: u64,
}

/// A worker's totals, carried over restarts (but `rare`, `new`, `seeds` and
/// `counters`, which belong to the process's pool and coverage, start over).
/// `signatures` counts the failure signatures it was first to find.
/// `counts` holds the oracles' tallies (`t.<oracle>.<field>`, comparisons
/// per variant as `v.<variant>`) and the mutators' fallbacks
/// (`fallbacks.file`, `fallbacks.query`).
#[derive(Default)]
struct Totals {
    counts: BTreeMap<String, u64>,
    /// The fallbacks of earlier processes, under this one's.
    base_fallbacks: (u64, u64),
    execs: u64,
    fresh: u64,
    mutants: u64,
    failures: u64,
    signatures: u64,
    timeouts: u64,
    gen_panics: u64,
    rare: u64,
    new: u64,
    seeds: u64,
    counters: u64,
    restarts: u64,
    by_oracle: [OracleTotals; ORACLES.len()],
}

impl Totals {
    fn load(path: &Path) -> Totals {
        let pairs: HashMap<String, u64> = read_pairs(path).into_iter().filter_map(|(k, v)| Some((k, v.parse().ok()?))).collect();
        let get = |k: &str| pairs.get(k).copied().unwrap_or(0);
        let mut by_oracle = [OracleTotals::default(); ORACLES.len()];
        for (o, (name, _)) in by_oracle.iter_mut().zip(ORACLES) {
            *o = OracleTotals {
                execs: get(&format!("o.{name}.execs")),
                failures: get(&format!("o.{name}.failures")),
                timeouts: get(&format!("o.{name}.timeouts")),
                ms: get(&format!("o.{name}.ms")),
            };
        }
        let counts: BTreeMap<String, u64> =
            pairs.iter().filter(|(k, _)| k.starts_with("t.") || k.starts_with("v.")).map(|(k, v)| (k.clone(), *v)).collect();
        Totals {
            counts,
            base_fallbacks: (get("fallbacks.file"), get("fallbacks.query")),
            execs: get("execs"),
            fresh: get("fresh"),
            mutants: get("mutants"),
            failures: get("failures"),
            signatures: get("signatures"),
            timeouts: get("timeouts"),
            gen_panics: get("gen_panics"),
            restarts: get("restarts") + u64::from(path.exists()),
            by_oracle,
            ..Totals::default()
        }
    }

    fn save(&self, path: &Path, n: u64, p: &Loop) {
        let mut text = format!(
            "mode={}\nprofile={}\nstorage={}\nn={n}\nexecs={}\nfresh={}\nmutants={}\nfailures={}\nsignatures={}\ntimeouts={}\ngen_panics={}\nrare={}\nnew={}\nseeds={}\ncounters={}\nrestarts={}\nheartbeat={}\n",
            p.mode(),
            p.name,
            p.storage.tag(),
            self.execs,
            self.fresh,
            self.mutants,
            self.failures,
            self.signatures,
            self.timeouts,
            self.gen_panics,
            self.rare,
            self.new,
            self.seeds,
            self.counters,
            self.restarts,
            now()
        );
        for (o, (name, _)) in self.by_oracle.iter().zip(ORACLES) {
            text.push_str(&format!("o.{name}.execs={}\no.{name}.failures={}\no.{name}.timeouts={}\no.{name}.ms={}\n", o.execs, o.failures, o.timeouts, o.ms));
        }
        for (k, v) in &self.counts {
            text.push_str(&format!("{k}={v}\n"));
        }
        text.push_str(&format!(
            "fallbacks.file={}\nfallbacks.query={}\n",
            self.base_fallbacks.0 + FILE_FALLBACKS.load(std::sync::atomic::Ordering::Relaxed),
            self.base_fallbacks.1 + QUERY_FALLBACKS.load(std::sync::atomic::Ordering::Relaxed)
        ));
        write_atomic(path, &text);
    }

    /// Add an oracle's tally on one input.
    fn add_tally(&mut self, oracle: &str, t: &Tally) {
        for (field, v) in [
            ("queries", t.queries),
            ("comparisons", t.comparisons),
            ("both_failed", t.both_failed),
            ("overflow_skips", t.overflow_skips),
            ("limit_skips", t.limit_skips),
            ("json_skips", t.json_skips),
            ("real_sum_skips", t.real_sum_skips),
            ("known_skips", t.known_skips),
            ("group_skips", t.group_skips),
            ("hint_skips", t.hint_skips),
            ("inapplicable", t.inapplicable),
            ("io_skips", t.io_skips),
            ("nondeterministic_skips", t.nondeterministic_skips),
        ] {
            if v > 0 {
                *self.counts.entry(format!("t.{oracle}.{field}")).or_default() += v;
            }
        }
        for (variant, v) in &t.variants {
            *self.counts.entry(format!("v.{variant}")).or_default() += v;
        }
    }

    /// The fields of a `worker` snapshot line.
    fn snapshot(&self, id: usize, p: &Loop, n: u64) -> String {
        let oracles: Vec<String> = self
            .by_oracle
            .iter()
            .zip(ORACLES)
            .map(|(o, (name, _))| format!("{}:{{\"execs\":{},\"failures\":{},\"timeouts\":{},\"ms\":{}}}", esc(name), o.execs, o.failures, o.timeouts, o.ms))
            .collect();
        format!(
            "\"kind\":\"worker\",\"worker\":{id},\"mode\":{},\"profile\":{},\"storage\":{},\"n\":{n},\"execs\":{},\"fresh\":{},\"mutants\":{},\"failures\":{},\"signatures\":{},\"timeouts\":{},\"gen_panics\":{},\"rare\":{},\"new\":{},\"seeds\":{},\"counters\":{},\"restarts\":{},\"by_oracle\":{{{}}}",
            esc(p.mode()),
            esc(&p.name),
            esc(p.storage.tag()),
            self.execs,
            self.fresh,
            self.mutants,
            self.failures,
            self.signatures,
            self.timeouts,
            self.gen_panics,
            self.rare,
            self.new,
            self.seeds,
            self.counters,
            self.restarts,
            oracles.join(",")
        )
    }
}

/// A `--queries LO-HI` range.
pub fn query_range(text: &str) -> Option<(usize, usize)> {
    let (lo, hi) = text.split_once('-')?;
    let (lo, hi): (usize, usize) = (lo.parse().ok()?, hi.parse().ok()?);
    (1 <= lo && lo <= hi).then_some((lo, hi))
}

/// Give `input`'s database k queries in all, k drawn log-uniformly from the
/// range by the input's seed `s`: more queries drawn (from a source of their
/// own, from `s`) on the case's final schema. An input with as many already,
/// or without tables, is left as it is.
pub fn extend(input: &mut Input, s: u64, size: usize, (lo, hi): (usize, usize)) {
    let u = (mix(s, 0x0e_7e4d) % 1_000_000) as f64 / 1_000_000.0;
    let k = ((lo as f64).ln() + u * ((hi as f64).ln() - (lo as f64).ln())).exp().round() as usize;
    let schema = final_schema(&input.0);
    if schema.tables.is_empty() || input.1.len() >= k {
        return;
    }
    let mut g = Source::new(mix(s, 0x0e_7e4e), size);
    while input.1.len() < k {
        let more = Queries(schema.clone()).generate(&mut g);
        if more.is_empty() {
            break;
        }
        input.1.extend(more);
    }
    input.1.truncate(k);
}

/// The deadline in seconds for an input of `queries` queries: `base`, and a
/// tenth of a second per query.
fn deadline(base: u64, queries: usize) -> u64 {
    base + queries as u64 / 10
}

fn panic_message(e: Box<dyn std::any::Any + Send>) -> String {
    e.downcast_ref::<String>().cloned().or(e.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default()
}

/// Run `oracle` on `input` in `storage` under a deadline of `secs`: its
/// tally, its failure, or `None` past the deadline. Panics are failures.
fn check(oracle: crate::Oracle, input: &Input, secs: u64, storage: &Storage) -> Option<Result<Tally, String>> {
    sqlancer::set_storage(storage.clone());
    sqlancer::set_deadline(Some(Instant::now() + Duration::from_secs(secs)));
    let verdict = match panic::catch_unwind(AssertUnwindSafe(|| oracle(&input.0, &input.1))) {
        Ok(r) => r,
        Err(e) => Err(format!("panic: {}", panic_message(e))),
    };
    let late = sqlancer::past_deadline();
    sqlancer::set_deadline(None);
    (!late).then_some(verdict)
}

/// The reference: `--draws` fresh draws (default [`REFERENCE`]) after a
/// warm-up, each under the memory oracle its index gives (of the first
/// [`MEMORY_ORACLES`], in memory), and how many reach each
/// counter, written to `D/reference.tsv` (its first line gives the draws).
pub fn reference(args: &[String]) {
    coverage::suppress_default_dump();
    panic::set_hook(Box::new(|_| {}));
    let dir = Path::new(&arg(args, "--dir").expect("--dir")).to_path_buf();
    let draws: u64 = arg(args, "--draws").map_or(REFERENCE, |s| s.parse().unwrap());
    let gen = case_queries();
    warm_up();
    let mut count: HashMap<usize, u32> = HashMap::new();
    for i in 0..draws {
        let Ok(v) = panic::catch_unwind(|| gen.generate(&mut Source::new(mix(REFERENCE_SEED, i), (i % 101) as usize))) else {
            continue;
        };
        let before = coverage::snapshot().expect("an instrumented SQLite");
        let _ = check(ORACLES[(i % MEMORY_ORACLES as u64) as usize].1, &v, 10, &Storage::Memory);
        for &(c, _) in before.delta(&coverage::snapshot().expect("an instrumented SQLite")).hits() {
            *count.entry(c).or_default() += 1;
        }
    }
    let mut out = format!("# {draws} draws\n");
    let mut sorted: Vec<_> = count.into_iter().collect();
    sorted.sort_unstable();
    for (c, k) in sorted {
        out.push_str(&format!("{c}\t{k}\n"));
    }
    write_atomic(&dir.join("reference.tsv"), &out);
}

/// A WAL loop's directory for its database files, `ephyra-wal-<run>-w<id>`
/// in `$SQLITE_TMPDIR` when it is set, else in `/tmp` (the first that is a
/// directory where it can be made; best a tmpfs), emptied of what an earlier
/// process left; or why none could be made.
pub fn wal_dir(run: &str, id: usize) -> Result<PathBuf, String> {
    wal_dir_under(std::env::var_os("SQLITE_TMPDIR"), run, id)
}

/// [`wal_dir`] with `tmpdir` for `$SQLITE_TMPDIR`.
fn wal_dir_under(tmpdir: Option<std::ffi::OsString>, run: &str, id: usize) -> Result<PathBuf, String> {
    let mut bases: Vec<PathBuf> = tmpdir.filter(|v| !v.is_empty()).map(PathBuf::from).into_iter().collect();
    bases.push(PathBuf::from("/tmp"));
    let run: String = run.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    let mut tried = Vec::new();
    for base in bases.into_iter().filter(|b| b.is_dir()) {
        let dir = base.join(format!("ephyra-wal-{run}-w{id}"));
        let _ = fs::remove_dir_all(&dir);
        match fs::create_dir_all(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) => tried.push(format!("{}: {e}", dir.display())),
        }
    }
    Err(format!("no directory for WAL files ({})", if tried.is_empty() { "neither $SQLITE_TMPDIR nor /tmp is a directory".into() } else { tried.join("; ") }))
}

/// The counters `seen` marks as a bitset: counter `c` is bit `c % 8` of byte
/// `c / 8`.
fn bitset(seen: &[bool]) -> Vec<u8> {
    let mut out = vec![0u8; seen.len().div_ceil(8)];
    for (c, _) in seen.iter().enumerate().filter(|(_, s)| **s) {
        out[c / 8] |= 1 << (c % 8);
    }
    out
}

pub fn run(args: &[String]) {
    coverage::suppress_default_dump();
    panic::set_hook(Box::new(|_| {}));
    let dir = Path::new(&arg(args, "--dir").expect("--dir")).to_path_buf();
    let id: usize = arg(args, "--id").expect("--id").parse().unwrap();
    let lp = match profiles::lookup(&arg(args, "--profile").or_else(|| arg(args, "--mode")).unwrap_or_else(|| "gen".into())) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        },
    };
    let (profile, mode, storage_tag) = (lp.profile, lp.mode(), lp.storage.tag());
    let name = lp.name.as_str();
    let seed: u64 = arg(args, "--seed").map_or(0, |s| s.parse().unwrap());
    let until: u64 = arg(args, "--until").map_or(u64::MAX, |s| s.parse().unwrap());
    let range = profile.queries.or_else(|| arg(args, "--queries").and_then(|r| query_range(&r)));
    let wdir = worker_dir(&dir, id);
    fs::create_dir_all(&wdir).unwrap();
    let (state_path, stats_path, reached_path) = (wdir.join("state"), wdir.join("stats"), wdir.join("reached"));

    // Carry on after the input in flight when the last process stopped.
    let state: HashMap<String, String> = read_pairs(&state_path).into_iter().collect();
    let mut n: u64 = state.get("n").and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut totals = Totals::load(&stats_path);
    totals.save(&stats_path, n, &lp);
    let ident = Identity::load(&dir).unwrap_or_else(|| Identity {
        run: "unknown".into(),
        campaign: "unknown".into(),
        tag: "untagged".into(),
        version: "unknown".into(),
        target: "unknown".into(),
    });
    let snapshots = dir.join("snapshots.jsonl");
    // A WAL loop's directory for its files.
    let storage = match lp.storage {
        StorageKind::Memory => Storage::Memory,
        StorageKind::Wal => match wal_dir(&ident.run, id) {
            Ok(d) => Storage::Wal { dir: d },
            Err(e) => {
                eprintln!("worker {id} ({name}): {e}");
                std::process::exit(2);
            },
        },
    };
    let wal_path = match &storage {
        Storage::Wal { dir } => esc(&dir.to_string_lossy()),
        Storage::Memory => "null".into(),
    };
    // Exit when the supervisor goes: then this process is someone else's
    // child (init's, or a container's or subreaper's).
    let parent = std::os::unix::process::parent_id();
    records::event(
        &dir,
        &ident.run,
        "worker_start",
        &format!(
            "\"worker\":{id},\"mode\":{},\"profile\":{},\"storage\":{},\"wal_dir\":{wal_path},\"pid\":{},\"at_input\":{}",
            esc(mode),
            esc(name),
            esc(storage_tag),
            std::process::id(),
            n + 1
        ),
        &format!("worker {id} ({name}) starts at input {}", n + 1),
    );

    let worker_seed = mix(seed, id as u64);
    // A profile that reads every input's counters runs SQLite's
    // once-per-process code first, so that its first inputs are not credited
    // with it.
    if profile.sample == 1 {
        warm_up();
        // And a WAL loop's once-per-process file, WAL and VFS code.
        if lp.storage == StorageKind::Wal {
            for i in 0..20 {
                let v = case_queries().generate(&mut Source::new(mix(0x0077_a2a1, i), i as usize));
                let _ = check(sqlancer::storage, &v, 10, &storage);
            }
            sqlancer::clear_storage();
        }
    }
    let mut seeds = Seeds::new(profile.pool, &dir);
    let mut saved_timeouts = 0;
    let mut seen: Vec<bool> = Vec::new();
    let profraw = wdir.join(format!("coverage-{}.profraw", now()));
    let statsd = arg(args, "--statsd").and_then(|addr| {
        let tags = arg(args, "--tags").unwrap_or_default();
        UdpSocket::bind("0.0.0.0:0").ok().map(|socket| Statsd { socket, addr, tags: format!("{tags},mode:{mode},profile:{name},storage:{storage_tag}") })
    });
    let (mut last_save, mut last_snapshot, mut last_profile) = (Instant::now(), Instant::now(), Instant::now());
    loop {
        if last_save.elapsed() >= Duration::from_secs(10) {
            totals.save(&stats_path, n, &lp);
            last_save = Instant::now();
        }
        if last_snapshot.elapsed() >= Duration::from_secs(30) {
            records::append(&snapshots, &ident.run, &totals.snapshot(id, &lp, n));
            write_atomic(&reached_path, bitset(&seen));
            last_snapshot = Instant::now();
        }
        if last_profile.elapsed() >= PROFILE_EVERY {
            save_profile(&profraw);
            last_profile = Instant::now();
        }
        if now() >= until || dir.join("stop").exists() || std::os::unix::process::parent_id() != parent {
            totals.save(&stats_path, n, &lp);
            records::append(&snapshots, &ident.run, &totals.snapshot(id, &lp, n));
            write_atomic(&reached_path, bitset(&seen));
            save_profile(&profraw);
            sqlancer::clear_storage();
            if let Storage::Wal { dir: d } = &storage {
                let _ = fs::remove_dir_all(d);
            }
            records::event(
                &dir,
                &ident.run,
                "worker_stop",
                &format!("\"worker\":{id},\"profile\":{},\"storage\":{},\"at_input\":{n}", esc(name), esc(storage_tag)),
                &format!("worker {id} stops at input {n}"),
            );
            return;
        }
        n += 1;
        let s = mix(worker_seed, n);
        let size = (n % 101) as usize;
        let oracles = lp.oracles(n);
        let queries = range.map_or(String::new(), |(lo, hi)| format!("queries={lo}-{hi}\n"));
        let state = |oracle: &str, mutant: bool| {
            write_atomic(
                &state_path,
                format!("n={n}\nseed={s}\nsize={size}\nmode={mode}\nprofile={name}\nstorage={storage_tag}\nmutant={mutant}\noracle={oracle}\nworker_seed={worker_seed}\n{queries}"),
            );
        };
        let planned = profile.is_mutant(n) && seeds.as_ref().is_some_and(|p| p.len() > 0);
        state(ORACLES[oracles[0]].0, planned);

        let drawn = panic::catch_unwind(AssertUnwindSafe(|| profiles::draw(profile, seeds.as_ref(), s, size, n, range)));
        let (input, parents) = match drawn {
            Ok(d) => d,
            Err(e) => {
                totals.gen_panics += 1;
                let msg = format!("generator panic (seed {s}, size {size}, profile {name}, storage {storage_tag}, mutant {planned}): {}", panic_message(e));
                let path = findings::record_lost(&dir, "generator-panics", id, &msg, "");
                records::event(
                    &dir,
                    &ident.run,
                    "generator_panic",
                    &format!(
                        "\"worker\":{id},\"profile\":{},\"input\":{n},\"seed\":{s},\"size\":{size},\"mutant\":{planned},\"path\":{}",
                        esc(name),
                        esc(&path.to_string_lossy())
                    ),
                    &msg,
                );
                continue;
            },
        };
        let mutant = !parents.is_empty();
        if mutant != planned {
            state(ORACLES[oracles[0]].0, mutant);
        }
        totals.execs += 1;
        if mutant {
            totals.mutants += 1;
        } else {
            totals.fresh += 1;
        }

        // Each oracle's counters are read before a failure is shrunk.
        let sampled = n.is_multiple_of(profile.sample);
        let (mut hits, mut timed_out, mut failed, mut near) = (Vec::new(), false, None, false);
        for (i, &o) in oracles.iter().enumerate() {
            let (oracle_name, oracle) = ORACLES[o];
            if i > 0 {
                state(oracle_name, mutant);
                if last_save.elapsed() >= Duration::from_secs(10) {
                    totals.save(&stats_path, n, &lp);
                    last_save = Instant::now();
                }
            }
            let before = sampled.then(|| coverage::snapshot().expect("an instrumented SQLite"));
            let t = Instant::now();
            let verdict = check(oracle, &input, deadline(10, input.1.len()), &storage);
            let elapsed = t.elapsed();
            if let Some(b) = before {
                hits.extend_from_slice(b.delta(&coverage::snapshot().expect("an instrumented SQLite")).hits());
            }
            totals.by_oracle[o].ms += elapsed.as_millis() as u64;
            totals.by_oracle[o].execs += 1;
            if let Some(sd) = &statsd {
                if n.is_multiple_of(LATENCY_SAMPLE) {
                    sd.distribution("input.ms", elapsed.as_secs_f64() * 1e3, 1.0 / LATENCY_SAMPLE as f64, &format!(",oracle:{oracle_name}"));
                }
            }
            match verdict {
                None => {
                    timed_out = true;
                    totals.timeouts += 1;
                    totals.by_oracle[o].timeouts += 1;
                    if saved_timeouts < SAVED_TIMEOUTS {
                        saved_timeouts += 1;
                        findings::record_timeout(&dir, &ident, id, name, storage_tag, n, oracle_name, &input);
                    }
                },
                Some(Err(message)) => {
                    totals.failures += 1;
                    totals.by_oracle[o].failures += 1;
                    let (shrunk, why) = shrink(&input, oracle, &storage, &message, &mut || {
                        totals.save(&stats_path, n, &lp);
                    });
                    if findings::record_failure(&dir, &ident, id, name, storage_tag, n, oracle_name, &why, &shrunk, Some((&message, &input))) {
                        totals.signatures += 1;
                    }
                    failed = Some(shrunk);
                },
                Some(Ok(tally)) => {
                    near |= profiles::near_miss(&tally);
                    totals.add_tally(oracle_name, &tally);
                },
            }
        }
        if let Some(sd) = &statsd {
            if n.is_multiple_of(SHAPE_SAMPLE) {
                let rate = 1.0 / SHAPE_SAMPLE as f64;
                for (name, v) in ast_metrics(&input.1).into_iter().filter(|(k, _)| SHAPE_QUERIES.contains(k)) {
                    sd.distribution(&format!("input.{}", name.replace(' ', "_")), v, rate, "");
                }
                for (name, v) in case_metrics(&input.0, 0).into_iter().filter(|(k, _)| SHAPE_CASE.contains(k)) {
                    sd.distribution(&format!("input.case_{}", name.replace(' ', "_")), v, rate, "");
                }
                sd.distribution("input.file_bytes", input.0.file.0.len() as f64, rate, "");
            }
        }
        let mut first = 0;
        for &(c, _) in &hits {
            if c >= seen.len() {
                seen.resize(c + 1, false);
            }
            if !seen[c] {
                seen[c] = true;
                first += 1;
            }
        }
        totals.counters += first as u64;
        if timed_out {
            continue;
        }
        match seeds.as_mut() {
            Some(Seeds::Rare(p)) => {
                p.observe(input, &hits);
                (totals.rare, totals.new) = (p.rare, p.new);
            },
            Some(Seeds::Novel(p)) => p.observe(input, first, &parents),
            Some(Seeds::NearMiss(p)) => {
                if let Some(f) = failed {
                    p.keep(f);
                } else if near {
                    p.keep(input);
                }
            },
            None => {},
        }
        if let Some(p) = &seeds {
            totals.seeds = p.len() as u64;
        }
    }
}

/// Rebuild a fresh draw (of any profile) from a crash or hang record's
/// state (the `seed=`, `size=`, `queries=`, `oracle=` and `storage=` lines
/// of `--state FILE`), write it as an example to `--out DIR`, and run its
/// oracle (without a deadline: a crash or a hang reproduces) in the same
/// storage (a WAL loop's in a WAL file in a directory of its own under the
/// system's temporary directory, removed after). Mutants are not rebuilt:
/// they came from the pool.
pub fn replay(args: &[String]) {
    let text = fs::read_to_string(arg(args, "--state").expect("--state")).unwrap();
    let pairs: HashMap<String, String> = text.lines().filter_map(|l| l.split_once('=')).map(|(k, v)| (k.into(), v.into())).collect();
    if pairs.get("mutant").map(String::as_str) == Some("true") {
        eprintln!("a mutant: it came from its worker's pool and cannot be rebuilt from its seed");
        std::process::exit(1);
    }
    let seed: u64 = pairs["seed"].parse().unwrap();
    let size: usize = pairs["size"].parse().unwrap();
    let (name, oracle) = *ORACLES.iter().find(|(n, _)| *n == pairs["oracle"]).expect("a known oracle");
    let mut input = case_queries().generate(&mut Source::new(seed, size));
    if let Some(r) = pairs.get("queries").and_then(|r| query_range(r)) {
        extend(&mut input, seed, size, r);
    }
    let wal = pairs.get("storage").map(String::as_str) == Some("wal");
    let storage = if wal { Storage::Wal { dir: std::env::temp_dir().join(format!("ephyra-wal-replay-{}", std::process::id())) } } else { Storage::Memory };
    let out = Path::new(&arg(args, "--out").expect("--out")).to_path_buf();
    let place = if wal { "in a WAL file" } else { "in memory" };
    findings::write_example(&out, &format!("replay of seed {seed}, size {size}, oracle {name}, {place}"), &input);
    println!("written to {}; running {name} {place}", out.display());
    sqlancer::set_storage(storage.clone());
    let verdict = oracle(&input.0, &input.1);
    sqlancer::clear_storage();
    if let Storage::Wal { dir } = &storage {
        let _ = fs::remove_dir_all(dir);
    }
    match verdict {
        Ok(_) => println!("passed"),
        Err(e) => println!("failed: {e}"),
    }
}

/// Shrink a failure greedily, in `storage`: the first candidate that fails
/// with the same first line, until none does, for at most 2 minutes;
/// `heartbeat` every 10 seconds.
fn shrink(input: &Input, oracle: crate::Oracle, storage: &Storage, message: &str, heartbeat: &mut dyn FnMut()) -> (Input, String) {
    let gen = case_queries();
    let kind = message.lines().next().unwrap_or("").to_string();
    let (mut best, mut why) = (input.clone(), message.to_string());
    let start = Instant::now();
    let mut beat = Instant::now();
    while start.elapsed() < Duration::from_secs(120) {
        let mut next = None;
        let _ = gen.shrink(&best, &mut |c| {
            if beat.elapsed() >= Duration::from_secs(10) {
                heartbeat();
                beat = Instant::now();
            }
            if start.elapsed() >= Duration::from_secs(120) {
                return ControlFlow::Break(());
            }
            match check(oracle, &c, deadline(5, c.1.len()), storage) {
                Some(Err(e)) if e.lines().next().unwrap_or("") == kind => {
                    next = Some((c, e));
                    ControlFlow::Break(())
                },
                _ => ControlFlow::Continue(()),
            }
        });
        match next {
            Some((c, e)) => {
                best = c;
                why = e;
            },
            None => break,
        }
    }
    (best, why)
}

#[cfg(test)]
mod tests {
    use ephyra::{Gen, Source};
    use ephyra_sqlite::{cases::Case, expr::Query, probes::case_queries, sqlancer::Tally};

    use ephyra_sqlite::sqlancer::Storage;

    use super::{check, shrink};
    use crate::findings;

    /// Fails whenever there is a query, as an oracle that found a bug would.
    fn fails_with_queries(_: &Case, queries: &[Query]) -> Result<Tally, String> {
        if queries.is_empty() {
            Ok(Tally::default())
        } else {
            Err(format!("results differ (fake)\n  {} queries", queries.len()))
        }
    }

    /// A failure shrinks to one query (the same first line throughout) and is
    /// recorded with everything a reproduction needs.
    #[test]
    fn failures_shrink_and_are_recorded() {
        let gen = case_queries();
        let input = (0..200)
            .map(|s| gen.generate(&mut Source::new(s, 60)))
            .find(|v| v.1.len() >= 2)
            .expect("an input with two queries");
        let Some(Err(message)) = check(fails_with_queries, &input, 5, &Storage::Memory) else {
            panic!("the fake oracle fails");
        };
        let (shrunk, why) = shrink(&input, fails_with_queries, &Storage::Memory, &message, &mut || {});
        assert_eq!(shrunk.1.len(), 1);
        assert!(why.starts_with("results differ (fake)"));

        let root = std::env::temp_dir().join(format!("runner-findings-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let ident = crate::records::Identity { run: "r".into(), campaign: "c".into(), tag: "t".into(), version: "v".into(), target: "x".into() };
        assert!(findings::record_failure(&root, &ident, 3, "gen+wal", "wal", 7, "fake", &why, &shrunk, Some((&message, &input))));
        assert!(!findings::record_failure(&root, &ident, 3, "gen+wal", "wal", 8, "fake", &why, &shrunk, Some((&message, &input))));
        let sig = root.join("findings").join(findings::signature("fake", &why));
        assert_eq!(std::fs::read_to_string(sig.join("count")).unwrap(), "2");
        let finding = std::fs::read_to_string(sig.join("finding.json")).unwrap();
        assert!(finding.contains("\"status\":\"open\"") && finding.contains("\"profile\":\"gen+wal\",\"storage\":\"wal\""), "{finding}");
        let example = std::fs::read_to_string(sig.join("w3-7").join("example.json")).unwrap();
        assert!(example.contains("\"storage\":\"wal\"") && example.contains("\"wal_settings\":{\"cache_size\":"), "{example}");
        assert!(std::fs::read_to_string(root.join("events.jsonl")).unwrap().lines().count() == 2);
        for f in ["message.txt", "input.txt", "queries.sql", "after.db", "before.db", "case.sql", "example.json"] {
            assert!(sig.join("w3-7").join(f).exists(), "{f}");
        }
        // Not a statement error: no unshrunk input.
        assert!(!sig.join("w3-7").join(findings::UNSHRUNK).exists());
        assert_eq!(findings::signatures(&root), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Fails as a case's statement would whenever there is a query.
    fn statement_fails_with_queries(_: &Case, queries: &[Query]) -> Result<Tally, String> {
        if queries.is_empty() {
            Ok(Tally::default())
        } else {
            Err(format!("statement 2: UPDATE t0 SET c1 = 1: no such column: c1\n  {} queries", queries.len()))
        }
    }

    /// A statement error's example also keeps the input before shrinking,
    /// in `unshrunk/`, with its own message; the example count is the
    /// examples' alone.
    #[test]
    fn statement_errors_keep_the_unshrunk_input() {
        let gen = case_queries();
        let input = (0..200)
            .map(|s| gen.generate(&mut Source::new(s, 60)))
            .find(|v| v.1.len() >= 2)
            .expect("an input with two queries");
        let Some(Err(message)) = check(statement_fails_with_queries, &input, 5, &Storage::Memory) else {
            panic!("the fake oracle fails");
        };
        let (shrunk, why) = shrink(&input, statement_fails_with_queries, &Storage::Memory, &message, &mut || {});
        assert_eq!(shrunk.1.len(), 1);

        let root = std::env::temp_dir().join(format!("runner-unshrunk-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let ident = crate::records::Identity { run: "r".into(), campaign: "c".into(), tag: "t".into(), version: "v".into(), target: "x".into() };
        assert!(findings::record_failure(&root, &ident, 1, "gen", "memory", 5, "fake", &why, &shrunk, Some((&message, &input))));
        let ex = root.join("findings").join(findings::signature("fake", &why)).join("w1-5");
        let un = ex.join(findings::UNSHRUNK);
        for f in ["message.txt", "input.txt", "queries.sql", "after.db", "before.db", "case.sql"] {
            assert!(un.join(f).exists(), "{f}");
        }
        assert!(!un.join("example.json").exists());
        let events = std::fs::read_to_string(root.join("events.jsonl")).unwrap();
        assert!(events.contains("\"unshrunk\":true"), "{events}");
        assert_eq!(std::fs::read_to_string(un.join("message.txt")).unwrap(), message);
        assert_eq!(std::fs::read_to_string(un.join("input.txt")).unwrap(), format!("{:?}\n{:?}\n", input.0, input.1));
        assert_eq!(std::fs::read_to_string(ex.join("input.txt")).unwrap(), format!("{:?}\n{:?}\n", shrunk.0, shrunk.1));
        // A second example: the signature has two, the unshrunk dirs not counted.
        assert!(!findings::record_failure(&root, &ident, 1, "gen", "memory", 6, "fake", &why, &shrunk, Some((&message, &input))));
        let sig = root.join("findings").join(findings::signature("fake", &why));
        assert_eq!(std::fs::read_dir(&sig).unwrap().filter_map(Result::ok).filter(|e| e.path().is_dir()).count(), 2);
        // Nothing shrunk: no copy.
        assert!(!findings::record_failure(&root, &ident, 1, "gen", "memory", 7, "fake", &message, &input, Some((&message, &input))));
        assert!(!sig.join("w1-7").join(findings::UNSHRUNK).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A WAL loop's directory is `$SQLITE_TMPDIR`'s (or `/tmp`'s), named
    /// after the run and the worker, and starts empty.
    #[test]
    fn wal_loops_get_a_fresh_directory() {
        let base = std::env::temp_dir().join(format!("runner-wal-dir-test-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let under = |t: &std::path::Path, run: &str, id: usize| super::wal_dir_under(Some(t.as_os_str().to_owned()), run, id);
        let dir = under(&base, "run/1 x", 4).unwrap();
        assert_eq!(dir, base.join("ephyra-wal-run_1_x-w4"));
        std::fs::write(dir.join("left-over.db"), b"x").unwrap();
        assert_eq!(under(&base, "run/1 x", 4).unwrap(), dir);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        // Without a usable $SQLITE_TMPDIR, /tmp.
        let id = std::process::id() as usize;
        for tmp in [under(&base.join("missing"), "runner-wal-dir-test", id).unwrap(), super::wal_dir_under(None, "runner-wal-dir-test", id).unwrap()] {
            assert_eq!(tmp, std::path::Path::new("/tmp").join(format!("ephyra-wal-runner-wal-dir-test-w{id}")));
            let _ = std::fs::remove_dir_all(&tmp);
        }
        let _ = std::fs::remove_dir_all(&base);
    }
}
