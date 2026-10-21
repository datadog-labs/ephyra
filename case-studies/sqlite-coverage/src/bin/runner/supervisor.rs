//! The supervisor: starts the workers, keeps them running, and reports.
//!
//! At its first start it writes the run's header (`D/run.json`, see
//! [`records`]); a restarted supervisor keeps it, and records a
//! `version_changed` event when its own code differs from the header's.
//! Workers get their loops ([`profiles`]: a profile on a storage) in order
//! from `--profiles NAME[+wal]:COUNT,..` (or else `--fresh N --targeted M`,
//! as `gen:N,rare:M`); `NAME+wal` is the profile with its database in a WAL
//! file (see [`crate::worker`]). All start at once but the `rare` ones
//! (`rare+wal` too), which start when their reference (`D/reference.tsv`,
//! computed by a child process beside the others, in memory, and only when
//! some worker is `rare`) is done.
//!
//! Every 5 seconds it looks at each worker. One that exited before the
//! deadline is restarted: by a signal it is a crash, with a non-zero code an
//! error, and either way the input in flight (its state file) is recorded
//! ([`findings::record_lost`]). One whose heartbeat (the mtime of its stats
//! file) is older than [`STALE`] (or `--stale-secs`) is killed, recorded as
//! a hang, and restarted, but never in its first `stale` after a start; one
//! over the memory limit is killed and restarted. A worker restarted more
//! than [`MAX_RESTARTS`] times in 10 minutes is given up. Each of these is an
//! event in `D/events.jsonl`.
//!
//! Every minute the workers' totals are summed into a `total` line of
//! `D/snapshots.jsonl` and a line of `D/stats.tsv`, and, with `--statsd`,
//! sent as DogStatsD metrics (`ephyra.sqlite.*`, tagged with the run, its
//! campaign, work tag, version and target, and `--tags`): counts of
//! executions, failures and timeouts since the last report (also per
//! oracle, tagged `oracle:<name>`), and gauges of workers alive, restarts,
//! crashes, hangs, signatures, executions per second, SQLite counters
//! reached (the most any worker has), and the `rare` workers' rare and new
//! counters. Per worker (tagged `worker`, `mode`, `profile` and `storage`):
//! its rate, memory and restarts. Per loop (tagged `profile`, the loop's
//! name with its `+wal`, and `storage`, `memory` or `wal`; also under
//! `by_profile` in the `total` line): its inputs (fresh and mutant), queries
//! checked, failures, timeouts and the signatures it found first, its seeds,
//! and the counters its workers reached (`ephyra.sqlite.profile.counters`,
//! the union of their `workers/K/reached` bitsets) and reached alone of all
//! loops (`profile.unique_counters`: a WAL loop's are mostly the file, WAL
//! and VFS code no memory loop runs). A `gen` worker reads one input's
//! counters in [`profiles::COVERAGE_SAMPLE`], so its sets, and the others'
//! unique counters against it, are of a sample; and a restarted worker's
//! set starts over. Crash and hang records and events carry the worker's
//! storage (its state file has it too). Datadog turns the `+` of a tag
//! value into `_`: there the WAL loops show as `gen_wal`, `novel_wal`.

use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Write as _,
    net::UdpSocket,
    os::unix::process::ExitStatusExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread::sleep,
    time::{Duration, Instant, SystemTime},
};

use crate::{
    arg, findings, now,
    profiles::{self, Loop, PoolKind},
    read_pairs,
    records::{self, esc},
    worker_dir, write_atomic, ORACLES,
};

/// A heartbeat older than this means a hang.
const STALE: Duration = Duration::from_secs(300);

/// Restarts of one worker within 10 minutes before it is given up.
const MAX_RESTARTS: usize = 10;

struct Worker {
    id: usize,
    lp: Loop,
    child: Option<Child>,
    /// When the current process started: its heartbeat is not judged
    /// before it has had `stale` to write one.
    spawned: Instant,
    restarts: Vec<Instant>,
    given_up: bool,
}

/// Totals over all workers, as their stats files give them: sums, except
/// `rare`, `new` and `counters`, the most any worker has.
#[derive(Default, Clone)]
struct Sum {
    values: BTreeMap<String, u64>,
}

impl Sum {
    fn get(&self, k: &str) -> u64 {
        self.values.get(k).copied().unwrap_or(0)
    }
}

fn sum<'a>(dir: &Path, workers: impl Iterator<Item = &'a Worker>) -> Sum {
    let mut s = Sum::default();
    for w in workers {
        for (k, v) in read_pairs(&worker_dir(dir, w.id).join("stats")) {
            let Ok(v) = v.parse::<u64>() else {
                continue;
            };
            if matches!(k.as_str(), "n" | "heartbeat" | "restarts") {
                continue;
            }
            let e = s.values.entry(k.clone()).or_default();
            if matches!(k.as_str(), "rare" | "new" | "counters") {
                *e = (*e).max(v);
            } else {
                *e += v;
            }
        }
    }
    s
}

/// What one loop's workers did: their storage, their totals ([`sum`]), the
/// queries the oracles checked, and the counters they reached (by their
/// `reached` files) and reached alone of all the loops.
#[derive(Default, Clone)]
struct ProfileSum {
    storage: &'static str,
    sum: Sum,
    queries: u64,
    counters: u64,
    unique: u64,
}

/// Each loop's [`ProfileSum`], by name (`gen`, `gen+wal`), in the order of
/// the workers.
fn by_profile(dir: &Path, workers: &[Worker]) -> Vec<(String, ProfileSum)> {
    let mut names: Vec<String> = Vec::new();
    for w in workers {
        if !names.contains(&w.lp.name) {
            names.push(w.lp.name.clone());
        }
    }
    // Each profile's union of its workers' counters, and the counters two
    // or more profiles reached.
    let bits: Vec<Vec<u8>> = names
        .iter()
        .map(|n| {
            let mut u: Vec<u8> = Vec::new();
            for w in workers.iter().filter(|w| w.lp.name == *n) {
                let b = fs::read(worker_dir(dir, w.id).join("reached")).unwrap_or_default();
                if b.len() > u.len() {
                    u.resize(b.len(), 0);
                }
                u.iter_mut().zip(&b).for_each(|(x, y)| *x |= y);
            }
            u
        })
        .collect();
    let (mut ones, mut twos) = (Vec::<u8>::new(), Vec::<u8>::new());
    for b in &bits {
        if b.len() > ones.len() {
            ones.resize(b.len(), 0);
            twos.resize(b.len(), 0);
        }
        for (i, &x) in b.iter().enumerate() {
            twos[i] |= ones[i] & x;
            ones[i] |= x;
        }
    }
    names
        .into_iter()
        .zip(&bits)
        .map(|(n, b)| {
            let storage = workers.iter().find(|w| w.lp.name == n).map_or("memory", |w| w.lp.storage.tag());
            let sum = sum(dir, workers.iter().filter(|w| w.lp.name == n));
            let queries = sum.values.iter().filter(|(k, _)| k.starts_with("t.") && k.ends_with(".queries")).map(|(_, v)| v).sum();
            let counters = b.iter().map(|x| u64::from(x.count_ones())).sum();
            let unique = b.iter().zip(&twos).map(|(x, t)| u64::from((x & !t).count_ones())).sum();
            (n, ProfileSum { storage, sum, queries, counters, unique })
        })
        .collect()
}

/// Resident memory of `pid` in MB, from `ps`.
fn rss_mb(pid: u32) -> u64 {
    Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<u64>().ok())
        .map_or(0, |kb| kb / 1024)
}

/// Where workers send their distributions: the DogStatsD address and the
/// run's tags, when there is one.
type Forward = Option<(String, String)>;

fn spawn(dir: &Path, run: &str, w: &Worker, seed: u64, until: u64, forward: &Forward, queries: &Option<String>) -> Option<Child> {
    let wdir = worker_dir(dir, w.id);
    fs::create_dir_all(&wdir).ok()?;
    let log = OpenOptions::new().create(true).append(true).open(wdir.join("log")).ok()?;
    let child = Command::new(std::env::current_exe().ok()?)
        .args(["worker", "--dir", dir.to_str()?, "--id", &w.id.to_string(), "--profile", &w.lp.name])
        .args(["--seed", &seed.to_string(), "--until", &until.to_string()])
        .args(forward.iter().flat_map(|(addr, tags)| ["--statsd".to_string(), addr.clone(), "--tags".to_string(), tags.clone()]))
        .args(queries.iter().flat_map(|q| ["--queries".to_string(), q.clone()]))
        .stdin(Stdio::null())
        .stdout(log.try_clone().ok()?)
        .stderr(log)
        .spawn()
        .ok()?;
    records::event(
        dir,
        run,
        "worker_spawn",
        &format!(
            "\"worker\":{},\"mode\":{},\"profile\":{},\"storage\":{},\"pid\":{}",
            w.id,
            esc(w.lp.mode()),
            esc(&w.lp.name),
            esc(w.lp.storage.tag()),
            child.id()
        ),
        &format!("worker {} ({}) started, pid {}", w.id, w.lp.name, child.id()),
    );
    Some(child)
}

fn heartbeat_age(dir: &Path, id: usize) -> Duration {
    fs::metadata(worker_dir(dir, id).join("stats"))
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .unwrap_or_default()
}

/// A value fixed at the run's first start (`D/<name>`), so that a restarted
/// supervisor keeps it.
fn fixed(dir: &Path, name: &str, first: impl FnOnce() -> u64) -> u64 {
    let path = dir.join(name);
    let v = fs::read_to_string(&path).ok().and_then(|s| s.trim().parse().ok()).unwrap_or_else(first);
    let _ = fs::write(&path, v.to_string());
    v
}

pub fn run(args: &[String]) {
    let dir = PathBuf::from(arg(args, "--dir").expect("--dir"));
    fs::create_dir_all(dir.join("findings")).unwrap();
    // `--profiles`, or else `--fresh N --targeted M` (gen and rare).
    let spec = arg(args, "--profiles").unwrap_or_else(|| {
        let count = |flag: &str, default: usize| arg(args, flag).map_or(default, |s| s.parse().unwrap());
        format!("gen:{},rare:{}", count("--fresh", 4), count("--targeted", 1))
    });
    let assigned = match profiles::parse(&spec) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        },
    };
    let fresh = assigned.iter().filter(|p| p.mode() == "fresh").count();
    let targeted = assigned.len() - fresh;
    let hours: f64 = arg(args, "--hours").map_or(24.0, |s| s.parse().unwrap());
    let max_rss_mb: u64 = arg(args, "--max-rss-mb").map_or(8192, |s| s.parse().unwrap());
    let stale = arg(args, "--stale-secs").map_or(STALE, |s| Duration::from_secs(s.parse().unwrap()));
    let draws = arg(args, "--reference").unwrap_or_else(|| "30000".into());
    let campaign = arg(args, "--campaign").unwrap_or_else(|| "sqlite/campaign/runner".into());
    let tag = arg(args, "--tag").unwrap_or_else(|| "untagged".into());
    let queries = arg(args, "--queries");
    // The known bugs whose fixes the target has (deploy/build-fixed.sh's
    // amalgamation, SQLITE_TARGET=fixed): recorded, so that the run says
    // which SQLite it tests and pre-triage knows its findings' target.
    let fixed_bugs: Vec<String> = arg(args, "--fixed-bugs").map_or_else(Vec::new, |s| s.split(',').filter(|b| !b.is_empty()).map(String::from).collect());
    let seed = fixed(&dir, "seed", || arg(args, "--seed").map_or_else(now, |s| s.parse().unwrap()));
    let until = fixed(&dir, "until", || now() + (hours * 3600.0) as u64);
    let config = format!(
        "{{\"profiles\":{},\"fresh\":{fresh},\"targeted\":{targeted},\"hours\":{hours},\"until\":{until},\"reference_draws\":{draws},\"stale_secs\":{},\"max_rss_mb\":{max_rss_mb},\"queries\":{},\"fixed_bugs\":[{}],\"oracles\":[{}]}}",
        esc(&spec),
        stale.as_secs(),
        queries.as_deref().map_or("null".into(), esc),
        fixed_bugs.iter().map(|b| esc(b)).collect::<Vec<_>>().join(","),
        ORACLES.iter().map(|(n, _)| esc(n)).collect::<Vec<_>>().join(",")
    );
    let (ident, changed) = records::header(&dir, &campaign, &tag, seed, &config);
    let run = ident.run.clone();
    let statsd = arg(args, "--statsd").and_then(|addr| UdpSocket::bind("0.0.0.0:0").ok().map(|s| (s, addr)));
    let mut tags = ident.tags();
    if let Some(t) = arg(args, "--tags") {
        tags.push(',');
        tags.push_str(&t);
    }
    let forward: Forward = statsd.as_ref().map(|(_, addr)| (addr.clone(), tags.clone()));
    let _ = fs::remove_file(dir.join("stop"));
    write_atomic(&dir.join("supervisor.pid"), std::process::id().to_string());
    records::event(
        &dir,
        &run,
        "run_start",
        &format!("\"pid\":{},\"version\":{},\"target\":{},\"config\":{config}", std::process::id(), esc(&ident.version), esc(&ident.target)),
        &format!("supervisor starts: run {run}, profiles {spec}, until {until}, seed {seed}, version {}", ident.version),
    );
    if records::built_version() != records::version().1 {
        records::event(
            &dir,
            &run,
            "stale_binary",
            &format!("\"built_version\":{},\"disk_version\":{}", esc(records::built_version()), esc(&records::version().1)),
            &format!("the binary was built from {}, not from the sources on disk ({})", records::built_version(), records::version().1),
        );
    }
    if changed {
        let (_, now_version) = records::version();
        records::event(
            &dir,
            &run,
            "version_changed",
            &format!("\"header_version\":{},\"running_version\":{}", esc(&ident.version), esc(&now_version)),
            &format!("the running code ({now_version}) differs from the run's header ({})", ident.version),
        );
    }

    // The `rare` workers' reference, computed beside the others; the `rare`
    // ones start when it is done.
    let mut reference: Option<Child> = None;
    if assigned.iter().any(|p| p.profile.pool == PoolKind::Rare) && !dir.join("reference.tsv").exists() {
        records::event(&dir, &run, "reference_start", &format!("\"draws\":{draws}"), "computing the reference");
        let log = File::create(dir.join("reference.log")).unwrap();
        reference = Command::new(std::env::current_exe().unwrap())
            .args(["reference", "--dir", dir.to_str().unwrap(), "--draws", &draws])
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .ok();
    }

    let mut workers: Vec<Worker> = assigned
        .into_iter()
        .enumerate()
        .map(|(id, lp)| Worker {
            id,
            lp,
            child: None,
            spawned: Instant::now(),
            restarts: Vec::new(),
            given_up: false,
        })
        .collect();
    for w in workers.iter_mut().filter(|w| w.lp.profile.pool != PoolKind::Rare || reference.is_none()) {
        w.child = spawn(&dir, &run, w, seed, until, &forward, &queries);
        w.spawned = Instant::now();
    }

    let mut tsv = OpenOptions::new().create(true).append(true).open(dir.join("stats.tsv")).unwrap();
    if tsv.metadata().map_or(0, |m| m.len()) == 0 {
        let _ = writeln!(tsv, "time\telapsed\talive\trestarts\tcrashes\thangs\texecs\texecs_per_sec\tfailures\ttimeouts\tgen_panics\tsignatures\tcounters\trare\tnew\tseeds");
    }
    let (mut crashes, mut hangs, mut restarts) = (0u64, 0u64, 0u64);
    let started = Instant::now();
    let mut last = (Instant::now(), sum(&dir, workers.iter()));
    let mut last_profiles: BTreeMap<String, ProfileSum> = by_profile(&dir, &workers).into_iter().collect();
    // Each worker's executions at the last report, for its rate.
    let mut last_execs: Vec<u64> = vec![0; workers.len()];
    loop {
        let stopping = now() >= until || dir.join("stop").exists();
        if let Some(r) = reference.as_mut() {
            if stopping {
                let _ = r.kill();
                let _ = r.wait();
                reference = None;
            } else if let Ok(Some(status)) = r.try_wait() {
                reference = None;
                let ok = status.success() && dir.join("reference.tsv").exists();
                records::event(&dir, &run, "reference_done", &format!("\"ok\":{ok}"), &format!("reference done: {status:?}"));
                if ok {
                    for w in workers.iter_mut().filter(|w| w.lp.profile.pool == PoolKind::Rare) {
                        w.child = spawn(&dir, &run, w, seed, until, &forward, &queries);
                        w.spawned = Instant::now();
                    }
                }
            }
        }
        for w in workers.iter_mut() {
            // `rare` workers wait for the reference (and are not started
            // without one).
            if w.lp.profile.pool == PoolKind::Rare && (reference.is_some() || !dir.join("reference.tsv").exists()) {
                continue;
            }
            let Some(child) = w.child.as_mut() else {
                continue;
            };
            let pid = child.id();
            let mut lost: Option<(&str, &str, String)> = None;
            match child.try_wait() {
                Ok(Some(status)) => {
                    w.child = None;
                    if stopping {
                        continue;
                    }
                    lost = Some(match (status.signal(), status.code()) {
                        (Some(sig), _) => ("crashes", "crash", format!("worker {} (pid {pid}) died of signal {sig}", w.id)),
                        (_, Some(code)) => ("errors", "error", format!("worker {} (pid {pid}) exited with code {code} before the deadline", w.id)),
                        _ => ("errors", "error", format!("worker {} (pid {pid}) exited: {status:?}", w.id)),
                    });
                },
                Ok(None) if !stopping && w.spawned.elapsed() > stale && heartbeat_age(&dir, w.id) > stale => {
                    let _ = child.kill();
                    let _ = child.wait();
                    w.child = None;
                    lost = Some(("hangs", "hang", format!("worker {} (pid {pid}) killed: no heartbeat for {} s", w.id, stale.as_secs())));
                },
                Ok(None) if !stopping && rss_mb(pid) > max_rss_mb => {
                    let _ = child.kill();
                    let _ = child.wait();
                    w.child = None;
                    records::event(
                        &dir,
                        &run,
                        "memory_kill",
                        &format!("\"worker\":{},\"pid\":{pid},\"limit_mb\":{max_rss_mb}", w.id),
                        &format!("worker {} (pid {pid}) killed: over {max_rss_mb} MB", w.id),
                    );
                },
                _ => {},
            }
            if let Some((folder, kind, detail)) = &lost {
                let state = fs::read_to_string(worker_dir(&dir, w.id).join("state")).unwrap_or_default();
                let path = findings::record_lost(&dir, folder, w.id, detail, &state);
                let state_json: Vec<String> = state.lines().filter_map(|l| l.split_once('=')).map(|(k, v)| format!("{}:{}", esc(k), esc(v))).collect();
                records::event(
                    &dir,
                    &run,
                    kind,
                    &format!(
                        "\"worker\":{},\"profile\":{},\"storage\":{},\"pid\":{pid},\"detail\":{},\"record\":{},\"state\":{{{}}}",
                        w.id,
                        esc(&w.lp.name),
                        esc(w.lp.storage.tag()),
                        esc(detail),
                        esc(&path.to_string_lossy()),
                        state_json.join(",")
                    ),
                    &format!("{detail}; recorded in {}", path.display()),
                );
                match *kind {
                    "crash" => crashes += 1,
                    "hang" => hangs += 1,
                    _ => {},
                }
            }
            if w.child.is_none() && !stopping && !w.given_up {
                w.restarts.retain(|t| t.elapsed() < Duration::from_secs(600));
                if w.restarts.len() >= MAX_RESTARTS {
                    w.given_up = true;
                    records::event(
                        &dir,
                        &run,
                        "worker_given_up",
                        &format!("\"worker\":{},\"restarts\":{MAX_RESTARTS}", w.id),
                        &format!("worker {} given up: {MAX_RESTARTS} restarts in 10 minutes", w.id),
                    );
                    continue;
                }
                w.restarts.push(Instant::now());
                restarts += 1;
                sleep(Duration::from_secs(1));
                w.child = spawn(&dir, &run, w, seed, until, &forward, &queries);
                w.spawned = Instant::now();
            }
        }

        let alive = workers.iter().filter(|w| w.child.is_some()).count() + usize::from(reference.is_some());
        let report = last.0.elapsed() >= Duration::from_secs(60) || (stopping && alive == 0);
        if report {
            let s = sum(&dir, workers.iter());
            let ps = by_profile(&dir, &workers);
            let secs = last.0.elapsed().as_secs_f64().max(1.0);
            let rate = s.get("execs").saturating_sub(last.1.get("execs")) as f64 / secs;
            let signatures = findings::signatures(&dir);
            let _ = writeln!(
                tsv,
                "{}\t{:.0}\t{alive}\t{restarts}\t{crashes}\t{hangs}\t{}\t{rate:.0}\t{}\t{}\t{}\t{signatures}\t{}\t{}\t{}\t{}",
                now(),
                started.elapsed().as_secs_f64(),
                s.get("execs"),
                s.get("failures"),
                s.get("timeouts"),
                s.get("gen_panics"),
                s.get("counters"),
                s.get("rare"),
                s.get("new"),
                s.get("seeds")
            );
            let oracles: Vec<String> = ORACLES
                .iter()
                .map(|(name, _)| {
                    let g = |k: &str| s.get(&format!("o.{name}.{k}"));
                    format!("{}:{{\"execs\":{},\"failures\":{},\"timeouts\":{},\"ms\":{}}}", esc(name), g("execs"), g("failures"), g("timeouts"), g("ms"))
                })
                .collect();
            let profiles: Vec<String> = ps
                .iter()
                .map(|(name, p)| {
                    let g = |k: &str| p.sum.get(k);
                    format!(
                        "{}:{{\"storage\":{},\"workers\":{},\"execs\":{},\"fresh\":{},\"mutants\":{},\"failures\":{},\"signatures\":{},\"timeouts\":{},\"queries\":{},\"seeds\":{},\"counters\":{},\"unique_counters\":{}}}",
                        esc(name),
                        esc(p.storage),
                        workers.iter().filter(|w| w.lp.name == *name).count(),
                        g("execs"),
                        g("fresh"),
                        g("mutants"),
                        g("failures"),
                        g("signatures"),
                        g("timeouts"),
                        p.queries,
                        g("seeds"),
                        p.counters,
                        p.unique
                    )
                })
                .collect();
            records::append(
                &dir.join("snapshots.jsonl"),
                &run,
                &format!(
                    "\"kind\":\"total\",\"alive\":{alive},\"restarts\":{restarts},\"crashes\":{crashes},\"hangs\":{hangs},\"execs\":{},\"execs_per_sec\":{rate:.1},\"failures\":{},\"timeouts\":{},\"gen_panics\":{},\"signatures\":{signatures},\"counters\":{},\"rare\":{},\"new\":{},\"seeds\":{},\"by_oracle\":{{{}}},\"by_profile\":{{{}}}",
                    s.get("execs"),
                    s.get("failures"),
                    s.get("timeouts"),
                    s.get("gen_panics"),
                    s.get("counters"),
                    s.get("rare"),
                    s.get("new"),
                    s.get("seeds"),
                    oracles.join(","),
                    profiles.join(",")
                ),
            );
            if let Some((socket, addr)) = &statsd {
                let d = |k: &str| s.get(k).saturating_sub(last.1.get(k));
                let mut lines = vec![
                    format!("ephyra.sqlite.execs:{}|c|#{tags}", d("execs")),
                    format!("ephyra.sqlite.failures:{}|c|#{tags}", d("failures")),
                    format!("ephyra.sqlite.timeouts:{}|c|#{tags}", d("timeouts")),
                    format!("ephyra.sqlite.generator_panics:{}|c|#{tags}", d("gen_panics")),
                    format!("ephyra.sqlite.execs_per_sec:{rate:.1}|g|#{tags}"),
                    format!("ephyra.sqlite.workers.alive:{alive}|g|#{tags}"),
                    format!("ephyra.sqlite.workers.restarts:{restarts}|g|#{tags}"),
                    format!("ephyra.sqlite.crashes:{crashes}|g|#{tags}"),
                    format!("ephyra.sqlite.hangs:{hangs}|g|#{tags}"),
                    format!("ephyra.sqlite.signatures:{signatures}|g|#{tags}"),
                    format!("ephyra.sqlite.coverage.counters:{}|g|#{tags}", s.get("counters")),
                    format!("ephyra.sqlite.targeted.rare:{}|g|#{tags}", s.get("rare")),
                    format!("ephyra.sqlite.targeted.new:{}|g|#{tags}", s.get("new")),
                ];
                // The oracles' tallies, comparisons per variant, inputs by
                // kind, and the mutators' fallbacks.
                for k in s.values.keys() {
                    let v = d(k);
                    if let Some(rest) = k.strip_prefix("t.") {
                        let Some((oracle, field)) = rest.split_once('.') else { continue };
                        match field {
                            "comparisons" | "queries" => lines.push(format!("ephyra.sqlite.oracle.{field}:{v}|c|#{tags},oracle:{oracle}")),
                            _ => lines.push(format!("ephyra.sqlite.oracle.skips:{v}|c|#{tags},oracle:{oracle},reason:{field}")),
                        }
                    } else if let Some(variant) = k.strip_prefix("v.") {
                        lines.push(format!("ephyra.sqlite.variant.comparisons:{v}|c|#{tags},variant:{variant}"));
                    } else if let Some(kind) = k.strip_prefix("fallbacks.") {
                        lines.push(format!("ephyra.sqlite.mutation.fallbacks:{v}|c|#{tags},kind:{kind}"));
                    }
                }
                lines.push(format!("ephyra.sqlite.inputs:{}|c|#{tags},kind:fresh", d("fresh")));
                lines.push(format!("ephyra.sqlite.inputs:{}|c|#{tags},kind:mutant", d("mutants")));
                // Each profile: its inputs (by kind), queries checked,
                // failures, timeouts and signatures it found first since the
                // last report, and the counters its workers reached, alone
                // too.
                for (name, p) in &ps {
                    let before = last_profiles.get(name).cloned().unwrap_or_default();
                    let d = |k: &str| p.sum.get(k).saturating_sub(before.sum.get(k));
                    let pt = format!("{tags},profile:{name},storage:{}", p.storage);
                    lines.push(format!("ephyra.sqlite.profile.execs:{}|c|#{pt}", d("execs")));
                    lines.push(format!("ephyra.sqlite.profile.inputs:{}|c|#{pt},kind:fresh", d("fresh")));
                    lines.push(format!("ephyra.sqlite.profile.inputs:{}|c|#{pt},kind:mutant", d("mutants")));
                    lines.push(format!("ephyra.sqlite.profile.queries:{}|c|#{pt}", p.queries.saturating_sub(before.queries)));
                    lines.push(format!("ephyra.sqlite.profile.failures:{}|c|#{pt}", d("failures")));
                    lines.push(format!("ephyra.sqlite.profile.timeouts:{}|c|#{pt}", d("timeouts")));
                    lines.push(format!("ephyra.sqlite.profile.signatures:{}|c|#{pt}", d("signatures")));
                    lines.push(format!("ephyra.sqlite.profile.seeds:{}|g|#{pt}", p.sum.get("seeds")));
                    lines.push(format!("ephyra.sqlite.profile.counters:{}|g|#{pt}", p.counters));
                    lines.push(format!("ephyra.sqlite.profile.unique_counters:{}|g|#{pt}", p.unique));
                }
                // Each worker: its rate, memory and restarts.
                for (i, w) in workers.iter().enumerate() {
                    let stats: std::collections::HashMap<String, u64> =
                        read_pairs(&worker_dir(&dir, w.id).join("stats")).into_iter().filter_map(|(k, v)| Some((k, v.parse().ok()?))).collect();
                    let execs = stats.get("execs").copied().unwrap_or(0);
                    let wt = format!("{tags},worker:{},mode:{},profile:{},storage:{}", w.id, w.lp.mode(), w.lp.name, w.lp.storage.tag());
                    lines.push(format!("ephyra.sqlite.worker.execs_per_sec:{:.1}|g|#{wt}", execs.saturating_sub(last_execs[i]) as f64 / secs));
                    lines.push(format!("ephyra.sqlite.worker.restarts:{}|g|#{wt}", stats.get("restarts").copied().unwrap_or(0)));
                    if let Some(c) = &w.child {
                        lines.push(format!("ephyra.sqlite.worker.rss_mb:{}|g|#{wt}", rss_mb(c.id())));
                    }
                    last_execs[i] = execs;
                }
                for (name, _) in ORACLES {
                    let o = |k: &str| d(&format!("o.{name}.{k}"));
                    lines.push(format!("ephyra.sqlite.oracle.execs:{}|c|#{tags},oracle:{name}", o("execs")));
                    lines.push(format!("ephyra.sqlite.oracle.failures:{}|c|#{tags},oracle:{name}", o("failures")));
                    lines.push(format!("ephyra.sqlite.oracle.timeouts:{}|c|#{tags},oracle:{name}", o("timeouts")));
                    lines.push(format!("ephyra.sqlite.oracle.ms:{}|c|#{tags},oracle:{name}", o("ms")));
                }
                for l in lines {
                    let _ = socket.send_to(l.as_bytes(), addr);
                }
            }
            last = (Instant::now(), s);
            last_profiles = ps.into_iter().collect();
        }
        if stopping && alive == 0 {
            records::event(&dir, &run, "run_stop", &format!("\"crashes\":{crashes},\"hangs\":{hangs},\"restarts\":{restarts}"), "supervisor stops: all workers done");
            let _ = fs::remove_file(dir.join("supervisor.pid"));
            return;
        }
        sleep(Duration::from_secs(5));
    }
}
