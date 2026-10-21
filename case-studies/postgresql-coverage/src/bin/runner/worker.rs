// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! A worker: inputs one after another on one session of the run's cluster,
//! until its deadline, the stop file, the supervisor's death (its parent
//! process changes), or the loss of its session.
//!
//! Input `n` of worker `K` is a fresh draw of `case_queries` from seed
//! `mix(mix(S, K), n)` at size `n % 101`, checked by `ORACLES[n % 10]` (the
//! model property, then norec, tlp, plans, order, setops, cte, window,
//! generic, views). Before it runs, `workers/K/state` names it (`n`, `seed`,
//! `size`, `oracle`, the worker's backend pid); a restarted worker carries
//! on from the input after it, with its totals. Every input reproduces from
//! its state (`runner replay`).
//!
//! What a check gives ([`Verdict`]):
//!
//! - a pass: the oracle's tally is added (queries, comparisons per variant,
//!   skips: inapplicable, plan-dependent errors, statement timeouts, known
//!   bugs);
//! - a failure: shrunk with the generator's shrinker (the first candidate
//!   with the same signature, until none has, for at most 2 minutes or
//!   `EPHYRA_RUNNER_SHRINK_SECS`), and recorded under its signature
//!   ([`findings::record_failure`]);
//! - a known PostgreSQL bug (`sqlancer::known_error`, where the oracles do
//!   not already skip it, e.g. in a case's statement): counted (`known`),
//!   not a finding;
//! - a statement timeout outside the oracles' own skips: counted
//!   (`timeouts`), the first few saved unshrunk under `findings/timeouts/`;
//! - the session lost (`pg::lost`, a closed connection, a server that
//!   refuses connections): the server crashed or was restarted. The worker
//!   writes `workers/K/lost` (the message and the input in flight; while
//!   shrinking, also `workers/K/lost.sql`, the candidate that ran) and exits
//!   with code [`LOST_EXIT`]; the supervisor records the crash and restarts
//!   it once the server is back.
//!
//! Every 10 seconds, and during shrinking, `workers/K/stats` is rewritten
//! (the totals and the heartbeat); every 30 seconds a `worker` line goes to
//! `D/snapshots.jsonl`. With `--statsd`, a worker sends DogStatsD
//! distributions: the time of one check in 10 (`input.ms`, tagged
//! `oracle`) and the shape of one input in 50 (`input.<metric>`, from
//! `probes::probe_metrics`).
//!
//! Test hook: with `EPHYRA_RUNNER_INJECT=<oracle>`, a pass of that oracle
//! on an input with at least one query is a failure ("injected failure"),
//! so the finding path (shrinking to one query, the record) runs without a
//! PostgreSQL bug. Never set it in a real run.
//!
//! Seam for coverage-guided loops (v1 has none): every backend of one
//! postmaster merges into one profile file, so one worker's coverage cannot
//! be read apart from the others'. A later version could give each guided
//! worker its own cluster (and `LLVM_PROFILE_FILE`), then choose inputs in
//! [`draw`] from a pool, as sqlite-coverage's `profiles.rs` does.

use std::{
    collections::{BTreeMap, HashMap},
    fs,
    net::UdpSocket,
    ops::ControlFlow,
    panic::{self, AssertUnwindSafe},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use ephyra::{Gen, Source};
use ephyra_postgresql::{
    pg::{self, close_session, with_session},
    probes::{case_queries, probe_metrics},
    sqlancer::{known_error, Tally},
};

use crate::{
    arg, findings, mix, now, read_pairs,
    records::{self, esc, Identity},
    script, worker_dir, write_atomic, Input, Oracle, ORACLES,
};

/// The exit code of a worker whose session was lost.
pub const LOST_EXIT: i32 = 3;

/// The exit code of a worker that renews its session: after
/// [`RENEW_EVERY`] inputs it saves its totals, closes its session (dropping
/// its database) and exits, and the supervisor starts it again at once, with
/// no record and no restart counted. A backend that lives for days gets
/// slower (2026-10-18: 69 inputs/s after 4 h, 37 after 33 h, with catalogs
/// vacuumed and the database small), so no session outlives a few hours.
pub const RENEW_EXIT: i32 = 4;
pub const RENEW_EVERY: u64 = 5000;

/// Timeouts saved per worker process (unshrunk).
const SAVED_TIMEOUTS: u64 = 5;

/// With `--statsd`, a worker sends the time of one check in
/// [`LATENCY_SAMPLE`] and the shape of one input in [`SHAPE_SAMPLE`].
const LATENCY_SAMPLE: u64 = 10;
const SHAPE_SAMPLE: u64 = 50;

/// A DogStatsD client: the worker's distributions.
struct Statsd {
    socket: UdpSocket,
    addr: String,
    tags: String,
}

impl Statsd {
    fn distribution(&self, name: &str, value: f64, rate: f64, extra: &str) {
        let line = format!("ephyra.postgresql.{name}:{value}|d|@{rate}|#{}{extra}", self.tags);
        let _ = self.socket.send_to(line.as_bytes(), &self.addr);
    }
}

/// One oracle's totals in a worker.
#[derive(Default, Clone, Copy)]
struct OracleTotals {
    execs: u64,
    failures: u64,
    timeouts: u64,
    known: u64,
    ms: u64,
}

/// A worker's totals, carried over restarts. `signatures` counts the
/// failure signatures it was first to find; `counts` holds the oracles'
/// tallies (`t.<oracle>.<field>`, comparisons per variant as `v.<variant>`).
#[derive(Default)]
struct Totals {
    counts: BTreeMap<String, u64>,
    execs: u64,
    failures: u64,
    signatures: u64,
    timeouts: u64,
    known: u64,
    gen_panics: u64,
    lost: u64,
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
                known: get(&format!("o.{name}.known")),
                ms: get(&format!("o.{name}.ms")),
            };
        }
        let counts = pairs.iter().filter(|(k, _)| k.starts_with("t.") || k.starts_with("v.")).map(|(k, v)| (k.clone(), *v)).collect();
        Totals {
            counts,
            execs: get("execs"),
            failures: get("failures"),
            signatures: get("signatures"),
            timeouts: get("timeouts"),
            known: get("known"),
            gen_panics: get("gen_panics"),
            lost: get("lost"),
            restarts: get("restarts") + u64::from(path.exists()),
            by_oracle,
        }
    }

    fn save(&self, path: &Path, n: u64, backend: i32) {
        let mut text = format!(
            "n={n}\nbackend={backend}\nexecs={}\nfailures={}\nsignatures={}\ntimeouts={}\nknown={}\ngen_panics={}\nlost={}\nrestarts={}\nheartbeat={}\n",
            self.execs,
            self.failures,
            self.signatures,
            self.timeouts,
            self.known,
            self.gen_panics,
            self.lost,
            self.restarts,
            now()
        );
        for (o, (name, _)) in self.by_oracle.iter().zip(ORACLES) {
            text.push_str(&format!(
                "o.{name}.execs={}\no.{name}.failures={}\no.{name}.timeouts={}\no.{name}.known={}\no.{name}.ms={}\n",
                o.execs, o.failures, o.timeouts, o.known, o.ms
            ));
        }
        for (k, v) in &self.counts {
            text.push_str(&format!("{k}={v}\n"));
        }
        write_atomic(path, &text);
    }

    /// Add an oracle's tally on one input.
    fn add_tally(&mut self, oracle: &str, t: &Tally) {
        for (field, v) in [
            ("queries", t.queries),
            ("comparisons", t.total()),
            ("inapplicable", t.inapplicable),
            ("error_skips", t.error_skips),
            ("timeouts", t.timeouts),
            ("known_skips", t.known_skips),
        ] {
            if v > 0 {
                *self.counts.entry(format!("t.{oracle}.{field}")).or_default() += v as u64;
            }
        }
        for (variant, v) in &t.comparisons {
            *self.counts.entry(format!("v.{variant}")).or_default() += *v as u64;
        }
    }

    /// The fields of a `worker` snapshot line.
    fn snapshot(&self, id: usize, n: u64) -> String {
        let oracles: Vec<String> = self
            .by_oracle
            .iter()
            .zip(ORACLES)
            .map(|(o, (name, _))| format!("{}:{{\"execs\":{},\"failures\":{},\"timeouts\":{},\"known\":{},\"ms\":{}}}", esc(name), o.execs, o.failures, o.timeouts, o.known, o.ms))
            .collect();
        format!(
            "\"kind\":\"worker\",\"worker\":{id},\"n\":{n},\"execs\":{},\"failures\":{},\"signatures\":{},\"timeouts\":{},\"known\":{},\"gen_panics\":{},\"lost\":{},\"restarts\":{},\"by_oracle\":{{{}}}",
            self.execs,
            self.failures,
            self.signatures,
            self.timeouts,
            self.known,
            self.gen_panics,
            self.lost,
            self.restarts,
            oracles.join(",")
        )
    }
}

/// What a check gave.
pub enum Verdict {
    Pass(Tally),
    Fail(String),
    Known(String),
    Timeout(String),
    Lost(String),
}

/// Whether an error means the session is gone: the server crashed, was
/// restarted, or refuses connections (in recovery, shutting down, not
/// there).
pub fn session_lost(e: &str) -> bool {
    pg::lost(e)
        || [
            "the server went away",
            "connect:",
            "Connection refused",
            "No such file or directory",
            "the database system is",
            "communicating with the server",
            "Broken pipe",
            "Connection reset",
            "unexpected EOF",
        ]
        .iter()
        .any(|m| e.contains(m))
}

fn statement_timeout(e: &str) -> bool {
    e.contains("statement timeout") || e.contains("(57014)")
}

fn panic_message(e: Box<dyn std::any::Any + Send>) -> String {
    e.downcast_ref::<String>().cloned().or(e.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default()
}

/// The oracle the test hook makes fail (`EPHYRA_RUNNER_INJECT`).
fn injected() -> Option<String> {
    std::env::var("EPHYRA_RUNNER_INJECT").ok().filter(|s| !s.is_empty())
}

/// Run `oracle` (named `name`) on `input` and classify the outcome. Panics
/// are failures.
pub fn check(name: &str, oracle: Oracle, input: &Input) -> Verdict {
    let r = match panic::catch_unwind(AssertUnwindSafe(|| oracle(&input.0, &input.1))) {
        Ok(r) => r,
        Err(e) => Err(format!("panic: {}", panic_message(e))),
    };
    match r {
        Ok(_) if injected().as_deref() == Some(name) && !input.1.is_empty() => {
            Verdict::Fail(format!("injected failure (test hook EPHYRA_RUNNER_INJECT)\n  {} queries", input.1.len()))
        },
        Ok(t) => Verdict::Pass(t),
        Err(e) if session_lost(&e) => Verdict::Lost(e),
        Err(e) if known_error(&e) => Verdict::Known(e),
        Err(e) if statement_timeout(&e) => Verdict::Timeout(e),
        Err(e) => Verdict::Fail(e),
    }
}

/// The input of worker seed `ws` at `n`: `(seed, size, input)`. A seam:
/// a guided loop would draw from a pool here.
fn draw(ws: u64, n: u64) -> (u64, usize, std::thread::Result<Input>) {
    let s = mix(ws, n);
    let size = (n % 101) as usize;
    (s, size, panic::catch_unwind(|| case_queries().generate(&mut Source::new(s, size))))
}

/// Shrink a failure greedily: the first candidate whose failure has the
/// same signature, until none does, for at most the budget; `heartbeat`
/// every 10 seconds. A candidate that loses the session ends shrinking with
/// it: `Err((candidate, message))`.
fn shrink(input: &Input, name: &str, oracle: Oracle, message: &str, heartbeat: &mut dyn FnMut()) -> Result<(Input, String), (Input, String)> {
    let budget = Duration::from_secs(std::env::var("EPHYRA_RUNNER_SHRINK_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(120));
    let gen = case_queries();
    let kind = findings::signature(name, message);
    let (mut best, mut why) = (input.clone(), message.to_string());
    let start = Instant::now();
    let mut beat = Instant::now();
    let mut lost: Option<(Input, String)> = None;
    while start.elapsed() < budget {
        let mut next = None;
        let _ = gen.shrink(&best, &mut |c| {
            if beat.elapsed() >= Duration::from_secs(10) {
                heartbeat();
                beat = Instant::now();
            }
            if start.elapsed() >= budget {
                return ControlFlow::Break(());
            }
            match check(name, oracle, &c) {
                Verdict::Fail(e) if findings::signature(name, &e) == kind => {
                    next = Some((c, e));
                    ControlFlow::Break(())
                },
                Verdict::Lost(e) => {
                    lost = Some((c, e));
                    ControlFlow::Break(())
                },
                _ => ControlFlow::Continue(()),
            }
        });
        if let Some(l) = lost {
            return Err(l);
        }
        match next {
            Some((c, e)) => {
                best = c;
                why = e;
            },
            None => break,
        }
    }
    Ok((best, why))
}

/// The worker's backend pid, connecting first; or why it could not.
fn connect() -> Result<i32, String> {
    with_session(|_| Ok(()))?;
    pg::backend_pid().ok_or_else(|| "no backend pid".to_string())
}

pub fn run(args: &[String]) {
    panic::set_hook(Box::new(|_| {}));
    let dir = Path::new(&arg(args, "--dir").expect("--dir")).to_path_buf();
    let id: usize = arg(args, "--id").expect("--id").parse().unwrap();
    let seed: u64 = arg(args, "--seed").map_or(0, |s| s.parse().unwrap());
    let until: u64 = arg(args, "--until").map_or(u64::MAX, |s| s.parse().unwrap());
    let wdir = worker_dir(&dir, id);
    fs::create_dir_all(&wdir).unwrap();
    let (state_path, stats_path, lost_path) = (wdir.join("state"), wdir.join("stats"), wdir.join("lost"));
    write_atomic(&wdir.join("pid"), std::process::id().to_string());
    let ident = Identity::load(&dir).unwrap_or_else(Identity::unknown);
    let snapshots = dir.join("snapshots.jsonl");

    // Carry on after the input in flight when the last process stopped.
    let state: HashMap<String, String> = read_pairs(&state_path).into_iter().collect();
    let mut n: u64 = state.get("n").and_then(|v| v.parse().ok()).unwrap_or(0);
    let first = n;
    // EPHYRA_RUNNER_RENEW_EVERY overrides RENEW_EVERY (tests).
    let renew_every: u64 = std::env::var("EPHYRA_RUNNER_RENEW_EVERY").ok().and_then(|v| v.parse().ok()).unwrap_or(RENEW_EVERY);
    let mut totals = Totals::load(&stats_path);
    let parent = std::os::unix::process::parent_id();
    let backend = match connect() {
        Ok(b) => b,
        Err(e) => {
            // The server is not there (yet): the supervisor tries again.
            totals.save(&stats_path, n, 0);
            write_atomic(&lost_path, format!("ts={}\nmessage={}\nat_connect=true\n", now(), e.replace('\n', " | ")));
            records::event(&dir, &ident.run, "worker_no_session", &format!("\"worker\":{id},\"error\":{}", esc(&e)), &format!("worker {id} cannot connect: {e}"));
            std::process::exit(LOST_EXIT);
        },
    };
    totals.save(&stats_path, n, backend);
    records::event(
        &dir,
        &ident.run,
        "worker_start",
        &format!("\"worker\":{id},\"pid\":{},\"backend_pid\":{backend},\"at_input\":{}", std::process::id(), n + 1),
        &format!("worker {id} starts at input {} (backend {backend})", n + 1),
    );

    let worker_seed = mix(seed, id as u64);
    let mut saved_timeouts = 0;
    let statsd = arg(args, "--statsd").and_then(|addr| {
        let tags = arg(args, "--tags").unwrap_or_default();
        UdpSocket::bind("0.0.0.0:0").ok().map(|socket| Statsd { socket, addr, tags: format!("{tags},worker:{id}") })
    });
    let (mut last_save, mut last_snapshot) = (Instant::now(), Instant::now());
    loop {
        if last_save.elapsed() >= Duration::from_secs(10) {
            totals.save(&stats_path, n, backend);
            last_save = Instant::now();
        }
        if last_snapshot.elapsed() >= Duration::from_secs(30) {
            records::append(&snapshots, &ident.run, &totals.snapshot(id, n));
            last_snapshot = Instant::now();
        }
        if now() >= until || dir.join("stop").exists() || std::os::unix::process::parent_id() != parent {
            totals.save(&stats_path, n, backend);
            records::append(&snapshots, &ident.run, &totals.snapshot(id, n));
            close_session();
            records::event(&dir, &ident.run, "worker_stop", &format!("\"worker\":{id},\"at_input\":{n}"), &format!("worker {id} stops at input {n}"));
            return;
        }
        if n - first >= renew_every {
            totals.save(&stats_path, n, backend);
            records::append(&snapshots, &ident.run, &totals.snapshot(id, n));
            close_session();
            records::event(&dir, &ident.run, "worker_renew", &format!("\"worker\":{id},\"at_input\":{n}"), &format!("worker {id} renews its session at input {n}"));
            std::process::exit(RENEW_EXIT);
        }
        n += 1;
        let o = (n % ORACLES.len() as u64) as usize;
        let (oracle_name, oracle) = ORACLES[o];
        let (s, size, drawn) = draw(worker_seed, n);
        let mut state = format!("n={n}\nseed={s}\nsize={size}\noracle={oracle_name}\nworker={id}\nworker_seed={worker_seed}\nbackend={backend}\nrun={}\n", ident.run);
        // What the generator lifted for this target: a replay rebuilds the
        // input with the same.
        if let Ok(f) = std::env::var("EPHYRA_FIXED_BUGS") {
            if !f.is_empty() {
                state.push_str(&format!("fixed={f}\n"));
            }
        }
        write_atomic(&state_path, &state);
        let input = match drawn {
            Ok(v) => v,
            Err(e) => {
                totals.gen_panics += 1;
                let msg = format!("generator panic (seed {s}, size {size}): {}", panic_message(e));
                let path = findings::record_lost(&dir, "generator-panics", &format!("w{id}"), &msg, &state);
                records::event(
                    &dir,
                    &ident.run,
                    "generator_panic",
                    &format!("\"worker\":{id},\"input\":{n},\"seed\":{s},\"size\":{size},\"path\":{}", esc(&path.to_string_lossy())),
                    &msg,
                );
                continue;
            },
        };
        totals.execs += 1;
        totals.by_oracle[o].execs += 1;
        let t = Instant::now();
        let verdict = check(oracle_name, oracle, &input);
        let elapsed = t.elapsed();
        totals.by_oracle[o].ms += elapsed.as_millis() as u64;
        if let Some(sd) = &statsd {
            if n.is_multiple_of(LATENCY_SAMPLE) {
                sd.distribution("input.ms", elapsed.as_secs_f64() * 1e3, 1.0 / LATENCY_SAMPLE as f64, &format!(",oracle:{oracle_name}"));
            }
            if n.is_multiple_of(SHAPE_SAMPLE) {
                for (name, v) in probe_metrics(&input, size) {
                    sd.distribution(&format!("input.{}", name.replace(' ', "_")), v, 1.0 / SHAPE_SAMPLE as f64, "");
                }
            }
        }
        let lose = |totals: &mut Totals, message: &str, candidate: Option<&Input>| {
            totals.lost += 1;
            totals.save(&stats_path, n, backend);
            let shrinking = candidate.is_some();
            write_atomic(&lost_path, format!("ts={}\nmessage={}\nshrinking={shrinking}\n{state}", now(), message.replace('\n', " | ")));
            if let Some(c) = candidate {
                write_atomic(&wdir.join("lost.sql"), script(c, &format!("candidate being checked (shrinking input {n}, {oracle_name}) when the session was lost\n{message}")));
            }
            records::event(
                &dir,
                &ident.run,
                "session_lost",
                &format!("\"worker\":{id},\"input\":{n},\"oracle\":{},\"shrinking\":{shrinking},\"message\":{}", esc(oracle_name), esc(message)),
                &format!("worker {id} lost its session at input {n} ({oracle_name}): {}", message.lines().next().unwrap_or("")),
            );
            std::process::exit(LOST_EXIT);
        };
        match verdict {
            Verdict::Pass(tally) => totals.add_tally(oracle_name, &tally),
            Verdict::Known(_) => {
                totals.known += 1;
                totals.by_oracle[o].known += 1;
            },
            Verdict::Timeout(message) => {
                totals.timeouts += 1;
                totals.by_oracle[o].timeouts += 1;
                if saved_timeouts < SAVED_TIMEOUTS {
                    saved_timeouts += 1;
                    let ex = dir.join("findings").join("timeouts").join(format!("w{id}-{n}"));
                    findings::write_example(&ex, &message, &input, &state);
                    records::event(&dir, &ident.run, "timeout_saved", &format!("\"worker\":{id},\"input\":{n},\"oracle\":{}", esc(oracle_name)), &format!("timeout saved (worker {id}, input {n}, {oracle_name})"));
                }
            },
            Verdict::Lost(message) => lose(&mut totals, &message, None),
            Verdict::Fail(message) => {
                totals.failures += 1;
                totals.by_oracle[o].failures += 1;
                write_atomic(&state_path, format!("{state}shrinking=true\n"));
                let shrunk = shrink(&input, oracle_name, oracle, &message, &mut || {
                    // A heartbeat only: the totals are saved after.
                    let _ = filetime_touch(&stats_path);
                });
                match shrunk {
                    Ok((best, why)) => {
                        if findings::record_failure(&dir, &ident, id, n, oracle_name, &why, &best, &state) {
                            totals.signatures += 1;
                        }
                        write_atomic(&state_path, &state);
                    },
                    Err((candidate, e)) => lose(&mut totals, &e, Some(&candidate)),
                }
            },
        }
    }
}

/// Refresh a file's modification time (the heartbeat) by rewriting it.
fn filetime_touch(path: &Path) -> std::io::Result<()> {
    let bytes = fs::read(path)?;
    let tmp = path.with_extension("beat");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}

/// The state a finding file holds: a directory's `state.txt`, a record's
/// part after "state (the input in flight):", or the file itself.
fn state_of(path: &Path) -> String {
    let file = if path.is_dir() { path.join("state.txt") } else { path.to_path_buf() };
    let text = fs::read_to_string(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
    match text.split_once("state (the input in flight):") {
        Some((_, rest)) => rest.to_string(),
        None => text,
    }
}

/// The run directory a path is inside (the nearest ancestor with
/// `run.json`).
fn run_dir_of(path: &Path) -> Option<PathBuf> {
    let abs = fs::canonicalize(path).ok()?;
    abs.ancestors().find(|a| a.join("run.json").exists()).map(Path::to_path_buf)
}

/// Rebuild an input from a finding (`--finding`: an example directory or
/// its `state.txt`, a crash's `w<K>.state.txt`, a hang, memory or lost
/// record), write it to `--out DIR` as `script.sql` (psql) and `input.txt`
/// (Debug), and run its oracle (or `--oracle`) once against the run's
/// cluster (`--dir D`, else the run directory the finding is in; it is
/// started if it is not running) or `--pg-dir`/`--pg-bin`. A failure's
/// example is rebuilt unshrunk (its shrunk script is beside it).
pub fn replay(args: &[String]) {
    panic::set_hook(Box::new(|_| {}));
    let finding = PathBuf::from(arg(args, "--finding").expect("--finding"));
    let out = PathBuf::from(arg(args, "--out").expect("--out"));
    let state = state_of(&finding);
    let pairs: HashMap<String, String> = crate::pairs(&state).into_iter().collect();
    // Before anything is generated: the run's fixed bugs change the input.
    std::env::set_var("EPHYRA_FIXED_BUGS", pairs.get("fixed").map_or("", |s| s.as_str()));
    let Some(input) = findings::rebuild(&state) else {
        eprintln!("no seed= and size= to rebuild an input from in {}", finding.display());
        std::process::exit(2);
    };
    let name = arg(args, "--oracle").or_else(|| pairs.get("oracle").cloned()).unwrap_or_else(|| "model".into());
    let Some(&(name, oracle)) = ORACLES.iter().find(|(n, _)| *n == name) else {
        eprintln!("unknown oracle {name}");
        std::process::exit(2);
    };
    let run_dir = arg(args, "--dir").map(PathBuf::from).or_else(|| run_dir_of(&finding));
    let pg_dir = arg(args, "--pg-dir").map(PathBuf::from).or_else(|| run_dir.as_ref().map(|d| crate::supervisor::pg_dir_of(d)));
    if let Some(d) = &pg_dir {
        std::env::set_var("EPHYRA_PG_DIR", d);
    }
    if let Some(b) = arg(args, "--pg-bin") {
        std::env::set_var("EPHYRA_PG_BIN", b);
    }
    let _ = fs::create_dir_all(&out);
    let header = format!("replay of seed {}, size {}, oracle {name}", pairs.get("seed").map_or("?", |s| s.as_str()), pairs.get("size").map_or("?", |s| s.as_str()));
    fs::write(out.join("script.sql"), script(&input, &header)).unwrap();
    fs::write(out.join("input.txt"), format!("{:?}\n{:?}\n", input.0, input.1)).unwrap();
    fs::write(out.join("state.txt"), &state).unwrap();
    let cluster = pg::pg_dir();
    println!("written to {}; running {name} on the cluster at {}", out.display(), cluster.display());
    let verdict = check(name, oracle, &input);
    close_session();
    let text = match verdict {
        Verdict::Pass(t) => format!("passed: {} queries, {} comparisons", t.queries, t.total()),
        Verdict::Fail(e) => format!("failed ({}): {e}", findings::signature(name, &e)),
        Verdict::Known(e) => format!("a known bug: {e}"),
        Verdict::Timeout(e) => format!("a statement timeout: {e}"),
        Verdict::Lost(e) => format!("the session was lost (a crash?): {e}"),
    };
    println!("{text}");
    let _ = fs::write(out.join("verdict.txt"), format!("{text}\n"));
    println!("psql: {} -X -h {} -U postgres -d postgres -f {}", pg::bin_dir().join("psql").display(), cluster.display(), out.join("script.sql").display());
}

#[cfg(test)]
mod tests {
    use ephyra::{Gen, Source};
    use ephyra_postgresql::{cases::Case, expr::Query, probes::case_queries, sqlancer::Tally};

    use super::{session_lost, shrink, Verdict};
    use crate::findings;

    fn fails_with_queries(_: &Case, queries: &[Query]) -> Result<Tally, String> {
        if queries.is_empty() {
            Ok(Tally::default())
        } else {
            Err(format!("fake: results differ\n  {} queries", queries.len()))
        }
    }

    /// A failure shrinks to one query without PostgreSQL (the fake oracle
    /// never connects) and is recorded with everything a reproduction
    /// needs.
    #[test]
    fn failures_shrink_and_are_recorded() {
        let gen = case_queries();
        let input = (0..200).map(|s| gen.generate(&mut Source::new(s, 60))).find(|v| v.1.len() >= 2).expect("an input with two queries");
        let Verdict::Fail(message) = super::check("fake", fails_with_queries, &input) else { panic!("the fake oracle fails") };
        let Ok((shrunk, why)) = shrink(&input, "fake", fails_with_queries, &message, &mut || {}) else { panic!("not lost") };
        assert_eq!(shrunk.1.len(), 1);
        assert!(why.starts_with("fake: results differ"));
        let root = std::env::temp_dir().join(format!("pg-runner-findings-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let ident = crate::records::Identity::unknown();
        assert!(findings::record_failure(&root, &ident, 3, 7, "fake", &why, &shrunk, "seed=1\nsize=2\n"));
        assert!(!findings::record_failure(&root, &ident, 3, 8, "fake", &why, &shrunk, "seed=1\nsize=2\n"));
        let sig = root.join("findings").join(findings::signature("fake", &why));
        assert_eq!(std::fs::read_to_string(sig.join("count")).unwrap(), "2");
        for f in ["message.txt", "input.txt", "script.sql", "state.txt", "example.json"] {
            assert!(sig.join("w3-7").join(f).exists(), "{f}");
        }
        assert_eq!(findings::signatures(&root), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn lost_sessions_are_told_apart() {
        assert!(session_lost("the server went away running SELECT 1: connection closed"));
        assert!(session_lost("connect: error connecting to server: No such file or directory (os error 2)"));
        assert!(session_lost("the database system is in recovery mode (57P03)"));
        assert!(!session_lost("division by zero (22012)"));
    }
}
