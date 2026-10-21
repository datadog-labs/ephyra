//! The supervisor: the run's cluster, the workers, and the reports.
//!
//! At its first start it writes the run's header (`D/run.json`, see
//! [`records`]); a restarted supervisor keeps it (and the seed, deadline,
//! counters and server-log offset in `D/seed`, `D/until`,
//! `D/supervisor.state`), records a `version_changed` event when its code
//! differs from the header's, and kills the workers a dead supervisor left
//! (`workers/K/pid`) before starting its own.
//!
//! **The cluster** (`D/pg`, or `--pg-dir`; binaries from `--pg-bin`) is
//! created and started with ephyra-postgresql's settings (`pg::ensure_server`:
//! initdb with the C locale, fsync off, autovacuum off, 200 connections),
//! through `EPHYRA_PG_DIR` and `EPHYRA_PG_BIN`, which the workers inherit.
//! Each worker is one session with a database of its own. It is stopped
//! (`pg_ctl stop -m fast`) when the run ends.
//!
//! **Every second** it reads what the server log (`server.log`) gained. A
//! crash (`TRAP:`, `PANIC:`, a server process "terminated by signal" or
//! "exited with exit code") makes the postmaster end every session and
//! reinitialize; the workers see their sessions lost and exit
//! ([`crate::worker::LOST_EXIT`], leaving `workers/K/lost`). The supervisor
//! then waits for them (15 s at most), records the crash
//! ([`findings::record_crash`], under its signature, with the log excerpt
//! and every worker's input in flight), waits for the server to accept
//! connections again (starting it when the postmaster is gone, and after
//! 3 minutes stopping it with `-m immediate` and starting it), and restarts
//! all workers. A restart caused only by the supervisor's own kills (a
//! backend over the memory limit, or a hung worker's backend) is an event
//! (`server_restart`), not a crash finding. A worker that lost its session
//! with no crash in the log is restarted after 20 s if the server answers,
//! else the server is handled as above.
//!
//! **Workers**: one that exits early is restarted (by a signal it is a
//! worker crash, the Rust client's, recorded under `findings/worker-crashes`;
//! another code an error); one whose heartbeat (the mtime of its stats file)
//! is older than `--stale-secs` (default [`STALE`]) is killed, recorded as a
//! hang, and its backend terminated (SIGTERM, then SIGKILL after 3 s: a
//! parse-analysis loop never reads interrupts); one over `--max-rss-mb` is
//! killed. A worker restarted more than [`MAX_RESTARTS`] times in 10 minutes
//! is given up.
//!
//! **Memory of backends**: a thread reads the process table every 500 ms
//! (`ps`), and SIGKILLs any client backend of the run's postmaster over
//! `--max-rss-mb` (a nested-BETWEEN query grows by more than 1 GB/s and no
//! statement timeout stops it), recording the worker whose backend it was
//! (`findings/memory/`, event `memory_kill`). That kill makes the server
//! restart, as above.
//!
//! **Every minute** the workers' totals are summed into a `total` line of
//! `D/snapshots.jsonl` and a line of `D/stats.tsv` and, with `--statsd`,
//! sent as DogStatsD metrics `ephyra.postgresql.*` (RUNNER.md lists them),
//! tagged with the run, campaign, work tag, version, target
//! (`postgresql-<commit>`) and `--tags`.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::{self, OpenOptions},
    io::{Read as _, Seek as _, SeekFrom, Write as _},
    net::UdpSocket,
    os::unix::process::ExitStatusExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    thread::sleep,
    time::{Duration, Instant, SystemTime},
};

use ephyra_postgresql::pg;

use crate::{
    arg, findings, now, read_pairs,
    records::{self, esc},
    worker::{LOST_EXIT, RENEW_EXIT},
    worker_dir, write_atomic, ORACLES,
};

/// A heartbeat older than this means a hang. A statement times out after
/// 10 s and a shrinking worker beats every 10 s, so 2 minutes is long.
const STALE: Duration = Duration::from_secs(120);

/// Restarts of one worker within 10 minutes before it is given up.
const MAX_RESTARTS: usize = 10;

/// The cluster directory of run `dir`: `D/pg_dir`'s contents when the run
/// was started with `--pg-dir`, else `D/pg`.
pub fn pg_dir_of(dir: &Path) -> PathBuf {
    fs::read_to_string(dir.join("pg_dir")).ok().map(|s| PathBuf::from(s.trim())).filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| dir.join("pg"))
}

struct Worker {
    id: usize,
    child: Option<Child>,
    /// When the current process started: its heartbeat is not judged
    /// before it has had `stale` to write one.
    spawned: Instant,
    restarts: Vec<Instant>,
    given_up: bool,
    /// It exited to renew its session (`RENEW_EXIT`): restart, uncounted.
    renew: bool,
    /// Since when it has been gone with its session lost.
    lost_since: Option<Instant>,
}

/// Sums of the workers' stats files (but `n`, `heartbeat`, `backend`).
#[derive(Default, Clone)]
struct Sum {
    values: BTreeMap<String, u64>,
}

impl Sum {
    fn get(&self, k: &str) -> u64 {
        self.values.get(k).copied().unwrap_or(0)
    }
}

fn sum(dir: &Path, workers: &[Worker]) -> Sum {
    let mut s = Sum::default();
    for w in workers {
        for (k, v) in read_pairs(&worker_dir(dir, w.id).join("stats")) {
            let Ok(v) = v.parse::<u64>() else { continue };
            if matches!(k.as_str(), "n" | "heartbeat" | "backend" | "restarts") {
                continue;
            }
            *s.values.entry(k).or_default() += v;
        }
    }
    s
}

/// A process: parent, resident MB, command line.
struct Proc {
    ppid: i32,
    rss_mb: u64,
    command: String,
}

/// The process table, from `ps`.
fn processes() -> HashMap<i32, Proc> {
    let Ok(out) = Command::new("ps").args(["-A", "-o", "pid=,ppid=,rss=,command="]).output() else {
        return HashMap::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let pid = it.next()?.parse().ok()?;
            let ppid = it.next()?.parse().ok()?;
            let rss: u64 = it.next()?.parse().ok()?;
            Some((pid, Proc { ppid, rss_mb: rss / 1024, command: it.collect::<Vec<_>>().join(" ") }))
        })
        .collect()
}

fn alive(pid: i32) -> bool {
    // SAFETY: kill with signal 0 only checks that the process exists.
    pid > 0 && unsafe { libc::kill(pid, 0) } == 0
}

fn signal(pid: i32, sig: i32) {
    if pid > 0 {
        // SAFETY: sending a signal to a pid has no memory-safety concerns.
        unsafe { libc::kill(pid, sig) };
    }
}

/// The postmaster's pid, from `postmaster.pid`.
fn postmaster(pg: &Path) -> Option<i32> {
    fs::read_to_string(pg.join("data").join("postmaster.pid")).ok()?.lines().next()?.trim().parse().ok()
}

/// `pg_isready`: whether the server accepts connections.
fn ready(pg: &Path) -> bool {
    Command::new(pg::bin_dir().join("pg_isready"))
        .args(["-q", "-t", "2", "-U", "postgres", "-d", "postgres", "-h"])
        .arg(pg)
        .status()
        .is_ok_and(|s| s.success())
}

/// What the memory thread shares with the supervisor.
#[derive(Default)]
struct Watch {
    /// Backends it killed: `(pid, MB)`, taken by the supervisor.
    killed: Vec<(i32, u64)>,
    /// Pids the supervisor killed itself: a server restart they cause is
    /// not a crash.
    expected: HashSet<i32>,
    backends: usize,
    backend_max_mb: u64,
    backend_total_mb: u64,
    /// Resident MB per backend pid, for per-worker metrics.
    rss: HashMap<i32, u64>,
}

/// Every 500 ms: SIGKILL any client backend of the cluster over the limit.
fn watch_memory(pg: PathBuf, limit_mb: u64, watch: Arc<Mutex<Watch>>) {
    loop {
        sleep(Duration::from_millis(500));
        let Some(pm) = postmaster(&pg) else { continue };
        let procs = processes();
        let (mut count, mut max, mut total, mut rss) = (0, 0, 0, HashMap::new());
        let mut kill = Vec::new();
        for (&pid, p) in procs.iter().filter(|(_, p)| p.ppid == pm) {
            count += 1;
            max = max.max(p.rss_mb);
            total += p.rss_mb;
            rss.insert(pid, p.rss_mb);
            // Client backends: "postgres: postgres <db> [local] <state>".
            if p.rss_mb > limit_mb && p.command.contains("[local]") {
                signal(pid, libc::SIGKILL);
                kill.push((pid, p.rss_mb));
            }
        }
        let mut w = watch.lock().unwrap_or_else(|e| e.into_inner());
        for &(pid, _) in &kill {
            w.expected.insert(pid);
        }
        w.killed.extend(kill);
        (w.backends, w.backend_max_mb, w.backend_total_mb, w.rss) = (count, max, total, rss);
    }
}

/// Where workers send their distributions: the DogStatsD address and the
/// run's tags.
type Forward = Option<(String, String)>;

fn spawn(dir: &Path, run: &str, w: &Worker, seed: u64, until: u64, forward: &Forward) -> Option<Child> {
    let wdir = worker_dir(dir, w.id);
    fs::create_dir_all(&wdir).ok()?;
    let _ = fs::remove_file(wdir.join("lost"));
    let _ = fs::remove_file(wdir.join("lost.sql"));
    let log = OpenOptions::new().create(true).append(true).open(wdir.join("log")).ok()?;
    let child = Command::new(std::env::current_exe().ok()?)
        .args(["worker", "--dir", dir.to_str()?, "--id", &w.id.to_string()])
        .args(["--seed", &seed.to_string(), "--until", &until.to_string()])
        .args(forward.iter().flat_map(|(addr, tags)| ["--statsd".to_string(), addr.clone(), "--tags".to_string(), tags.clone()]))
        .stdin(Stdio::null())
        .stdout(log.try_clone().ok()?)
        .stderr(log)
        .spawn()
        .ok()?;
    records::event(dir, run, "worker_spawn", &format!("\"worker\":{},\"pid\":{}", w.id, child.id()), &format!("worker {} started, pid {}", w.id, child.id()));
    Some(child)
}

fn heartbeat_age(dir: &Path, id: usize) -> Duration {
    fs::metadata(worker_dir(dir, id).join("stats"))
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .unwrap_or_default()
}

fn rss_mb(pid: u32) -> u64 {
    Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<u64>().ok())
        .map_or(0, |kb| kb / 1024)
}

/// A value fixed at the run's first start (`D/<name>`).
fn fixed(dir: &Path, name: &str, first: impl FnOnce() -> u64) -> u64 {
    let path = dir.join(name);
    let v = fs::read_to_string(&path).ok().and_then(|s| s.trim().parse().ok()).unwrap_or_else(first);
    let _ = fs::write(&path, v.to_string());
    v
}

/// The supervisor's counters, kept over its restarts.
#[derive(Default)]
struct Counters {
    crashes: u64,
    hangs: u64,
    restarts: u64,
    memory_kills: u64,
    server_restarts: u64,
    worker_crashes: u64,
    log_offset: u64,
}

impl Counters {
    fn load(path: &Path) -> Counters {
        let p: HashMap<String, u64> = read_pairs(path).into_iter().filter_map(|(k, v)| Some((k, v.parse().ok()?))).collect();
        let g = |k: &str| p.get(k).copied().unwrap_or(0);
        Counters {
            crashes: g("crashes"),
            hangs: g("hangs"),
            restarts: g("restarts"),
            memory_kills: g("memory_kills"),
            server_restarts: g("server_restarts"),
            worker_crashes: g("worker_crashes"),
            log_offset: g("log_offset"),
        }
    }

    fn save(&self, path: &Path) {
        write_atomic(
            path,
            format!(
                "crashes={}\nhangs={}\nrestarts={}\nmemory_kills={}\nserver_restarts={}\nworker_crashes={}\nlog_offset={}\n",
                self.crashes, self.hangs, self.restarts, self.memory_kills, self.server_restarts, self.worker_crashes, self.log_offset
            ),
        );
    }
}

/// What the server log gained since `offset` (whole lines only), and the
/// offset after it.
fn log_since(log: &Path, offset: u64) -> (String, u64) {
    let Ok(mut f) = fs::File::open(log) else { return (String::new(), offset) };
    let len = f.metadata().map_or(0, |m| m.len());
    // A new log (the cluster recreated): from its start.
    let offset = if len < offset { 0 } else { offset };
    let mut buf = Vec::new();
    if f.seek(SeekFrom::Start(offset)).is_err() || f.take(16 << 20).read_to_end(&mut buf).is_err() {
        return (String::new(), offset);
    }
    let end = buf.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
    buf.truncate(end);
    (String::from_utf8_lossy(&buf).into_owned(), offset + end as u64)
}

/// Whether a log line shows a server process crashing: an assertion, a
/// PANIC, a process terminated by a signal or exiting with a code other
/// than 0 and 1 (a parallel worker's or a backend's ERROR or FATAL exits
/// with 1, which the postmaster logs and is no crash), or the postmaster
/// reinitializing.
pub fn crash_line(l: &str) -> bool {
    let abnormal_exit = l.split_once("exited with exit code ").is_some_and(|(_, c)| !matches!(c.split_whitespace().next(), Some("0" | "1")));
    abnormal_exit || ["TRAP:", "PANIC:", "terminated by signal", "was terminated by exception", "all server processes terminated"].iter().any(|m| l.contains(m))
}

/// Whether log text has a [`crash_line`].
fn crash_lines(text: &str) -> bool {
    text.lines().any(crash_line)
}

/// The pids of server processes the log says were terminated by a signal
/// or exited abnormally.
fn terminated_pids(text: &str) -> Vec<i32> {
    text.lines()
        .filter(|l| crash_line(l) && (l.contains("terminated by signal") || l.contains("exited with exit code")))
        .filter_map(|l| l.split_once("(PID ")?.1.split(')').next()?.parse().ok())
        .collect()
}

/// The last `max` lines of `text`.
fn tail(text: &str, max: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(max)..].join("\n")
}

struct Supervisor {
    dir: PathBuf,
    pg: PathBuf,
    run: String,
    seed: u64,
    until: u64,
    forward: Forward,
    workers: Vec<Worker>,
    counters: Counters,
    watch: Arc<Mutex<Watch>>,
    state_path: PathBuf,
}

impl Supervisor {
    fn event(&self, kind: &str, fields: &str, text: &str) {
        records::event(&self.dir, &self.run, kind, fields, text);
    }

    fn spawn(&mut self, i: usize) {
        let w = &self.workers[i];
        let child = spawn(&self.dir, &self.run, w, self.seed, self.until, &self.forward);
        let w = &mut self.workers[i];
        w.child = child;
        w.spawned = Instant::now();
        w.lost_since = None;
    }

    fn state(&self, id: usize) -> String {
        fs::read_to_string(worker_dir(&self.dir, id).join("state")).unwrap_or_default()
    }

    /// Start the cluster (creating it the first time); waits for it.
    fn start_server(&self) -> Result<(), String> {
        let r = pg::ensure_server();
        if r.is_ok() {
            self.event("server_start", &format!("\"postmaster_pid\":{}", postmaster(&self.pg).unwrap_or(0)), &format!("cluster at {} running, postmaster {}", self.pg.display(), postmaster(&self.pg).unwrap_or(0)));
        }
        r
    }

    /// The server crashed, was restarted, or is gone (`reason`): wait for
    /// the workers' sessions to be lost, record what happened, bring the
    /// server back, restart every worker.
    fn server_trouble(&mut self, reason: &str) {
        let started = Instant::now();
        // The workers exit when they see their sessions lost.
        while started.elapsed() < Duration::from_secs(15) && self.workers.iter_mut().any(|w| w.child.as_mut().is_some_and(|c| matches!(c.try_wait(), Ok(None)))) {
            sleep(Duration::from_millis(250));
        }
        sleep(Duration::from_secs(1));
        let log_path = self.pg.join("server.log");
        let (text, end) = log_since(&log_path, self.counters.log_offset);
        self.counters.log_offset = end;
        let killed: Vec<i32> = terminated_pids(&text);
        let mut watch = self.watch.lock().unwrap_or_else(|e| e.into_inner());
        let ours = !killed.is_empty() && killed.iter().all(|p| watch.expected.contains(p)) && !text.contains("TRAP:") && !text.contains("PANIC:");
        watch.expected.clear();
        drop(watch);
        // Every worker's input in flight: its lost file (the state when the
        // session went) or its state file.
        let mut in_flight = Vec::new();
        for w in &self.workers {
            let wdir = worker_dir(&self.dir, w.id);
            let lost = fs::read_to_string(wdir.join("lost")).ok().filter(|l| !l.contains("at_connect=true"));
            in_flight.push((w.id, lost.unwrap_or_else(|| self.state(w.id))));
        }
        if ours {
            self.counters.server_restarts += 1;
            self.event(
                "server_restart",
                &format!("\"reason\":\"supervisor_kill\",\"pids\":{:?}", killed),
                &format!("server restarted after the supervisor killed backend(s) {killed:?}"),
            );
        } else if crash_lines(&text) || reason != "log" {
            let marks: Vec<&str> = text.lines().filter(|l| crash_line(l) || l.contains("Failed process was running")).collect();
            let excerpt = format!("-- crash lines --\n{}\n-- the log's last 300 lines --\n{}", marks.join("\n"), tail(&text, 300));
            // The workers whose backend the log names (terminated, or the
            // TRAP line's PID).
            let named: Vec<i32> = killed.iter().copied().chain(text.lines().filter_map(|l| l.split_once(", PID: ")?.1.trim().parse().ok())).collect();
            let culprits: Vec<usize> = in_flight
                .iter()
                .filter(|(_, st)| st.lines().find_map(|l| l.strip_prefix("backend=")).and_then(|b| b.parse::<i32>().ok()).is_some_and(|b| named.contains(&b)))
                .map(|(w, _)| *w)
                .collect();
            let (sig, new, at) = findings::record_crash(&self.dir, &excerpt, &in_flight, &culprits);
            for w in &self.workers {
                let candidate = worker_dir(&self.dir, w.id).join("lost.sql");
                if candidate.exists() {
                    let _ = fs::copy(&candidate, at.join(format!("w{}.candidate.sql", w.id)));
                }
            }
            self.counters.crashes += 1;
            self.counters.server_restarts += 1;
            let first = excerpt.lines().find(|l| l.contains("TRAP:") || l.contains("PANIC:") || l.contains("terminated by signal")).unwrap_or(reason);
            self.event(
                "crash",
                &format!(
                    "\"signature\":{},\"new_signature\":{new},\"reason\":{},\"record\":{},\"first_line\":{},\"workers_in_flight\":{},\"culprits\":{culprits:?}",
                    esc(&sig),
                    esc(reason),
                    esc(&at.to_string_lossy()),
                    esc(first),
                    in_flight.len()
                ),
                &format!("server crash {sig} ({}): recorded in {}", if new { "new signature" } else { "repeat" }, at.display()),
            );
        }
        // Kill what is left of the workers.
        for w in self.workers.iter_mut() {
            if let Some(c) = w.child.as_mut() {
                let _ = c.kill();
                let _ = c.wait();
            }
            w.child = None;
        }
        // The server back: crash recovery, else a start.
        let wait = Instant::now();
        let mut restarted = false;
        while !ready(&self.pg) {
            if !postmaster(&self.pg).is_some_and(alive) || (wait.elapsed() > Duration::from_secs(180) && !restarted) {
                if postmaster(&self.pg).is_some_and(alive) {
                    let _ = Command::new(pg::bin_dir().join("pg_ctl")).arg("-D").arg(self.pg.join("data")).args(["stop", "-m", "immediate", "-w"]).output();
                }
                restarted = true;
                self.event("server_down", &format!("\"reason\":{}", esc(reason)), "postmaster gone: starting the cluster");
                if let Err(e) = self.start_server() {
                    self.event("server_start_failed", &format!("\"error\":{}", esc(&e)), &format!("cannot start the cluster: {e}"));
                }
            }
            if wait.elapsed() > Duration::from_secs(600) {
                self.event("server_unrecoverable", "", "the cluster did not come back in 10 minutes; stopping the run");
                write_atomic(&self.dir.join("stop"), "server unrecoverable\n");
                break;
            }
            sleep(Duration::from_secs(1));
        }
        self.event(
            "server_ready",
            &format!("\"recovery_secs\":{},\"postmaster_pid\":{}", started.elapsed().as_secs(), postmaster(&self.pg).unwrap_or(0)),
            &format!("server ready again after {} s", started.elapsed().as_secs()),
        );
        let (_, end) = log_since(&log_path, self.counters.log_offset);
        self.counters.log_offset = end;
        self.counters.save(&self.state_path);
        if !self.dir.join("stop").exists() && now() < self.until {
            for i in 0..self.workers.len() {
                if !self.workers[i].given_up {
                    self.spawn(i);
                }
            }
        }
    }

    /// Kill a hung worker's backend: SIGTERM, then SIGKILL (expected) when it
    /// is still there 3 s later.
    fn kill_backend(&self, backend: i32) {
        if backend <= 0 || !alive(backend) {
            return;
        }
        signal(backend, libc::SIGTERM);
        for _ in 0..12 {
            sleep(Duration::from_millis(250));
            if !alive(backend) {
                return;
            }
        }
        self.watch.lock().unwrap_or_else(|e| e.into_inner()).expected.insert(backend);
        signal(backend, libc::SIGKILL);
    }
}

/// Kill the workers a dead supervisor left running.
fn kill_leftovers(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir.join("workers")) else { return };
    let procs = processes();
    for e in entries.filter_map(Result::ok) {
        let Some(pid) = fs::read_to_string(e.path().join("pid")).ok().and_then(|s| s.trim().parse::<i32>().ok()) else { continue };
        if procs.get(&pid).is_some_and(|p| p.command.contains(" worker ") && p.command.contains(&dir.to_string_lossy().to_string())) {
            signal(pid, libc::SIGKILL);
        }
    }
}

pub fn run(args: &[String]) {
    let dir = PathBuf::from(arg(args, "--dir").expect("--dir"));
    fs::create_dir_all(dir.join("findings")).unwrap();
    let dir = fs::canonicalize(&dir).unwrap();
    let workers_n: usize = arg(args, "--workers").map_or(4, |s| s.parse().unwrap());
    let hours: f64 = arg(args, "--hours").map_or(24.0, |s| s.parse().unwrap());
    let max_rss_mb: u64 = arg(args, "--max-rss-mb").map_or(4096, |s| s.parse().unwrap());
    let stale = arg(args, "--stale-secs").map_or(STALE, |s| Duration::from_secs(s.parse().unwrap()));
    let campaign = arg(args, "--campaign").unwrap_or_else(|| "postgresql/campaign/runner".into());
    let tag = arg(args, "--tag").unwrap_or_else(|| "untagged".into());
    if let Some(b) = arg(args, "--pg-bin") {
        std::env::set_var("EPHYRA_PG_BIN", b);
    }
    // The known bugs the target has fixed: the workers inherit it, and with
    // it what ephyra_postgresql::fixed lifts.
    let fixed_bugs = arg(args, "--fixed-bugs").unwrap_or_default();
    std::env::set_var("EPHYRA_FIXED_BUGS", &fixed_bugs);
    if let Some(p) = arg(args, "--pg-dir") {
        let _ = fs::create_dir_all(&p);
        write_atomic(&dir.join("pg_dir"), fs::canonicalize(&p).unwrap().to_string_lossy().as_bytes());
    }
    let pg_dir = pg_dir_of(&dir);
    std::env::set_var("EPHYRA_PG_DIR", &pg_dir);
    let socket = pg_dir.join(".s.PGSQL.5432");
    if socket.as_os_str().len() >= 100 {
        eprintln!("the cluster's socket path {} is too long for a Unix socket; use a shorter --dir or --pg-dir", socket.display());
        std::process::exit(2);
    }
    let seed = fixed(&dir, "seed", || arg(args, "--seed").map_or_else(now, |s| s.parse().unwrap()));
    let until = fixed(&dir, "until", || now() + (hours * 3600.0) as u64);
    let config = format!(
        "{{\"workers\":{workers_n},\"hours\":{hours},\"until\":{until},\"stale_secs\":{},\"max_rss_mb\":{max_rss_mb},\"pg_dir\":{},\"pg_bin\":{},\"fixed_bugs\":{},\"oracles\":[{}]}}",
        stale.as_secs(),
        esc(&pg_dir.to_string_lossy()),
        esc(&pg::bin_dir().to_string_lossy()),
        esc(&fixed_bugs),
        ORACLES.iter().map(|(n, _)| esc(n)).collect::<Vec<_>>().join(",")
    );
    let (ident, changed) = records::header(&dir, &campaign, &tag, seed, &config, &pg::bin_dir());
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
        &format!("supervisor starts: run {run}, {workers_n} workers, until {until}, seed {seed}, version {}, cluster {}", ident.version, pg_dir.display()),
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
        let now_version = records::version().1;
        records::event(
            &dir,
            &run,
            "version_changed",
            &format!("\"header_version\":{},\"running_version\":{}", esc(&ident.version), esc(&now_version)),
            &format!("the running code ({now_version}) differs from the run's header ({})", ident.version),
        );
    }
    kill_leftovers(&dir);

    let state_path = dir.join("supervisor.state");
    let watch = Arc::new(Mutex::new(Watch::default()));
    let mut sv = Supervisor {
        dir: dir.clone(),
        pg: pg_dir.clone(),
        run: run.clone(),
        seed,
        until,
        forward,
        workers: (0..workers_n).map(|id| Worker { id, child: None, spawned: Instant::now(), restarts: Vec::new(), given_up: false, renew: false, lost_since: None }).collect(),
        counters: Counters::load(&state_path),
        watch: watch.clone(),
        state_path: state_path.clone(),
    };
    if let Err(e) = sv.start_server() {
        sv.event("server_start_failed", &format!("\"error\":{}", esc(&e)), &format!("cannot start the cluster: {e}"));
        eprintln!("cannot start the cluster: {e}");
        std::process::exit(1);
    }
    {
        let (pg, w) = (pg_dir.clone(), watch.clone());
        std::thread::spawn(move || watch_memory(pg, max_rss_mb, w));
    }
    if now() < until {
        for i in 0..workers_n {
            sv.spawn(i);
        }
    }

    let mut tsv = OpenOptions::new().create(true).append(true).open(dir.join("stats.tsv")).unwrap();
    if tsv.metadata().map_or(0, |m| m.len()) == 0 {
        let _ = writeln!(
            tsv,
            "time\telapsed\talive\trestarts\tcrashes\thangs\tmemory_kills\tserver_restarts\texecs\texecs_per_sec\tfailures\ttimeouts\tknown\tgen_panics\tlost\tsignatures\tcrash_signatures\tbackends\tbackend_max_mb"
        );
    }
    let started = Instant::now();
    let mut last = (Instant::now(), sum(&dir, &sv.workers));
    let mut last_execs: Vec<u64> = vec![0; workers_n];
    let mut last_rss = Instant::now();
    let mut stop_since: Option<Instant> = None;
    loop {
        let stopping = now() >= until || dir.join("stop").exists();
        if stopping && stop_since.is_none() {
            stop_since = Some(Instant::now());
            sv.event("run_stopping", &format!("\"deadline\":{}", now() >= until), "stopping: waiting for the workers");
        }

        // Backends the memory thread killed.
        let killed: Vec<(i32, u64)> = std::mem::take(&mut watch.lock().unwrap_or_else(|e| e.into_inner()).killed);
        for (pid, mb) in killed {
            let owner = sv.workers.iter().find(|w| read_pairs(&worker_dir(&dir, w.id).join("stats")).iter().any(|(k, v)| k == "backend" && v.parse() == Ok(pid))).map(|w| w.id);
            let state = owner.map(|id| sv.state(id)).unwrap_or_default();
            let detail = format!("backend {pid} of worker {} killed: {mb} MB, over {max_rss_mb} MB", owner.map_or("?".into(), |w| w.to_string()));
            let path = findings::record_lost(&dir, "memory", &owner.map_or(format!("backend{pid}"), |w| format!("w{w}")), &detail, &state);
            sv.counters.memory_kills += 1;
            sv.event(
                "memory_kill",
                &format!("\"process\":\"backend\",\"backend_pid\":{pid},\"worker\":{},\"rss_mb\":{mb},\"limit_mb\":{max_rss_mb},\"record\":{}", owner.map_or("null".into(), |w| w.to_string()), esc(&path.to_string_lossy())),
                &format!("{detail}; recorded in {}", path.display()),
            );
        }

        // The server: a crash in its log, or a postmaster gone.
        if !stopping || sv.workers.iter().any(|w| w.child.is_some()) {
            let (text, end) = log_since(&pg_dir.join("server.log"), sv.counters.log_offset);
            if crash_lines(&text) {
                sv.server_trouble("log");
                continue;
            }
            sv.counters.log_offset = end;
            if !postmaster(&pg_dir).is_some_and(alive) && !stopping {
                sv.server_trouble("postmaster gone");
                continue;
            }
        }

        let check_rss = last_rss.elapsed() >= Duration::from_secs(5);
        if check_rss {
            last_rss = Instant::now();
        }
        let mut trouble = None;
        for i in 0..sv.workers.len() {
            let id = sv.workers[i].id;
            let w = &mut sv.workers[i];
            let mut lost: Option<(&str, &str, String)> = None;
            let mut backend_to_kill = 0;
            if let Some(child) = w.child.as_mut() {
                let pid = child.id();
                match child.try_wait() {
                    Ok(Some(status)) => {
                        w.child = None;
                        if stopping {
                            continue;
                        }
                        match (status.signal(), status.code()) {
                            (_, Some(LOST_EXIT)) => w.lost_since = Some(Instant::now()),
                            (_, Some(RENEW_EXIT)) => w.renew = true,
                            (Some(sig), _) => lost = Some(("worker-crashes", "worker_crash", format!("worker {id} (pid {pid}) died of signal {sig}"))),
                            (_, Some(code)) => lost = Some(("errors", "error", format!("worker {id} (pid {pid}) exited with code {code} before the deadline"))),
                            _ => lost = Some(("errors", "error", format!("worker {id} (pid {pid}) exited: {status:?}"))),
                        }
                    },
                    Ok(None) if !stopping && w.spawned.elapsed() > stale && heartbeat_age(&dir, id) > stale => {
                        let _ = child.kill();
                        let _ = child.wait();
                        w.child = None;
                        backend_to_kill = read_pairs(&worker_dir(&dir, id).join("stats")).iter().find(|(k, _)| k == "backend").and_then(|(_, v)| v.parse().ok()).unwrap_or(0);
                        lost = Some(("hangs", "hang", format!("worker {id} (pid {pid}) killed: no heartbeat for {} s; its backend {backend_to_kill} terminated", stale.as_secs())));
                    },
                    Ok(None) if !stopping && check_rss && rss_mb(pid) > max_rss_mb => {
                        let _ = child.kill();
                        let _ = child.wait();
                        w.child = None;
                        lost = Some(("memory", "memory_kill", format!("worker {id} (pid {pid}) killed: over {max_rss_mb} MB")));
                    },
                    _ => {},
                }
            }
            let w = &sv.workers[i];
            if let Some((folder, kind, detail)) = &lost {
                let state = sv.state(id);
                let path = findings::record_lost(&dir, folder, &format!("w{id}"), detail, &state);
                let state_json: Vec<String> = state.lines().filter_map(|l| l.split_once('=')).map(|(k, v)| format!("{}:{}", esc(k), esc(v))).collect();
                sv.event(
                    kind,
                    &format!("\"worker\":{id},\"detail\":{},\"record\":{},\"state\":{{{}}}", esc(detail), esc(&path.to_string_lossy()), state_json.join(",")),
                    &format!("{detail}; recorded in {}", path.display()),
                );
                match *kind {
                    "hang" => sv.counters.hangs += 1,
                    "worker_crash" => sv.counters.worker_crashes += 1,
                    "memory_kill" => sv.counters.memory_kills += 1,
                    _ => {},
                }
                sv.kill_backend(backend_to_kill);
            }
            // A lost session with no crash in the log.
            if let Some(since) = w.lost_since {
                if since.elapsed() > Duration::from_secs(20) {
                    if ready(&pg_dir) {
                        let reason = fs::read_to_string(worker_dir(&dir, id).join("lost")).unwrap_or_default();
                        let message = reason.lines().find_map(|l| l.strip_prefix("message=")).unwrap_or("").to_string();
                        sv.event("worker_lost", &format!("\"worker\":{id},\"message\":{}", esc(&message)), &format!("worker {id} lost its session without a crash in the log: {message}"));
                        sv.workers[i].lost_since = None;
                    } else {
                        trouble = Some("server not ready after a worker lost its session");
                    }
                }
            }
            let w = &mut sv.workers[i];
            if w.child.is_none() && w.renew && !stopping {
                w.renew = false;
                sv.spawn(i);
                continue;
            }
            if w.child.is_none() && w.lost_since.is_none() && !stopping && !w.given_up {
                w.restarts.retain(|t| t.elapsed() < Duration::from_secs(600));
                if w.restarts.len() >= MAX_RESTARTS {
                    w.given_up = true;
                    sv.event("worker_given_up", &format!("\"worker\":{id},\"restarts\":{MAX_RESTARTS}"), &format!("worker {id} given up: {MAX_RESTARTS} restarts in 10 minutes"));
                    continue;
                }
                w.restarts.push(Instant::now());
                sv.counters.restarts += 1;
                sv.spawn(i);
            }
        }
        if let Some(reason) = trouble {
            sv.server_trouble(reason);
            continue;
        }
        // Past the stop, workers get 90 s to finish their input.
        if stop_since.is_some_and(|t| t.elapsed() > Duration::from_secs(90)) {
            for w in sv.workers.iter_mut() {
                if let Some(c) = w.child.as_mut() {
                    let _ = c.kill();
                    let _ = c.wait();
                    w.child = None;
                }
            }
        }

        let alive_n = sv.workers.iter().filter(|w| w.child.is_some()).count();
        let report = last.0.elapsed() >= Duration::from_secs(60) || (stopping && alive_n == 0);
        if report {
            let s = sum(&dir, &sv.workers);
            let secs = last.0.elapsed().as_secs_f64().max(1.0);
            let rate = s.get("execs").saturating_sub(last.1.get("execs")) as f64 / secs;
            let signatures = findings::signatures(&dir);
            let crash_signatures = findings::crash_signatures(&dir);
            let (backends, backend_max, backend_total, backend_rss) = {
                let w = watch.lock().unwrap_or_else(|e| e.into_inner());
                (w.backends, w.backend_max_mb, w.backend_total_mb, w.rss.clone())
            };
            let c = &sv.counters;
            let _ = writeln!(
                tsv,
                "{}\t{:.0}\t{alive_n}\t{}\t{}\t{}\t{}\t{}\t{}\t{rate:.1}\t{}\t{}\t{}\t{}\t{}\t{signatures}\t{crash_signatures}\t{backends}\t{backend_max}",
                now(),
                started.elapsed().as_secs_f64(),
                c.restarts,
                c.crashes,
                c.hangs,
                c.memory_kills,
                c.server_restarts,
                s.get("execs"),
                s.get("failures"),
                s.get("timeouts"),
                s.get("known"),
                s.get("gen_panics"),
                s.get("lost")
            );
            let oracles: Vec<String> = ORACLES
                .iter()
                .map(|(name, _)| {
                    let g = |k: &str| s.get(&format!("o.{name}.{k}"));
                    format!(
                        "{}:{{\"execs\":{},\"failures\":{},\"timeouts\":{},\"known\":{},\"ms\":{},\"queries\":{},\"comparisons\":{}}}",
                        esc(name),
                        g("execs"),
                        g("failures"),
                        g("timeouts"),
                        g("known"),
                        g("ms"),
                        s.get(&format!("t.{name}.queries")),
                        s.get(&format!("t.{name}.comparisons"))
                    )
                })
                .collect();
            records::append(
                &dir.join("snapshots.jsonl"),
                &run,
                &format!(
                    "\"kind\":\"total\",\"alive\":{alive_n},\"restarts\":{},\"crashes\":{},\"hangs\":{},\"memory_kills\":{},\"server_restarts\":{},\"worker_crashes\":{},\"execs\":{},\"execs_per_sec\":{rate:.1},\"failures\":{},\"timeouts\":{},\"known\":{},\"gen_panics\":{},\"lost\":{},\"signatures\":{signatures},\"crash_signatures\":{crash_signatures},\"backends\":{backends},\"backend_max_mb\":{backend_max},\"backend_total_mb\":{backend_total},\"by_oracle\":{{{}}}",
                    c.restarts,
                    c.crashes,
                    c.hangs,
                    c.memory_kills,
                    c.server_restarts,
                    c.worker_crashes,
                    s.get("execs"),
                    s.get("failures"),
                    s.get("timeouts"),
                    s.get("known"),
                    s.get("gen_panics"),
                    s.get("lost"),
                    oracles.join(",")
                ),
            );
            if let Some((socket, addr)) = &statsd {
                let d = |k: &str| s.get(k).saturating_sub(last.1.get(k));
                let p = "ephyra.postgresql";
                let mut lines = vec![
                    format!("{p}.execs:{}|c|#{tags}", d("execs")),
                    format!("{p}.failures:{}|c|#{tags}", d("failures")),
                    format!("{p}.timeouts:{}|c|#{tags}", d("timeouts")),
                    format!("{p}.known_bugs:{}|c|#{tags}", d("known")),
                    format!("{p}.generator_panics:{}|c|#{tags}", d("gen_panics")),
                    format!("{p}.sessions_lost:{}|c|#{tags}", d("lost")),
                    format!("{p}.inputs:{}|c|#{tags},kind:fresh", d("execs")),
                    format!("{p}.execs_per_sec:{rate:.1}|g|#{tags}"),
                    format!("{p}.workers.alive:{alive_n}|g|#{tags}"),
                    format!("{p}.workers.restarts:{}|g|#{tags}", c.restarts),
                    format!("{p}.crashes:{}|g|#{tags}", c.crashes),
                    format!("{p}.hangs:{}|g|#{tags}", c.hangs),
                    format!("{p}.memory_kills:{}|g|#{tags}", c.memory_kills),
                    format!("{p}.server_restarts:{}|g|#{tags}", c.server_restarts),
                    format!("{p}.worker_crashes:{}|g|#{tags}", c.worker_crashes),
                    format!("{p}.signatures:{signatures}|g|#{tags}"),
                    format!("{p}.crash_signatures:{crash_signatures}|g|#{tags}"),
                    format!("{p}.backends.count:{backends}|g|#{tags}"),
                    format!("{p}.backends.max_rss_mb:{backend_max}|g|#{tags}"),
                    format!("{p}.backends.total_rss_mb:{backend_total}|g|#{tags}"),
                ];
                for k in s.values.keys() {
                    let v = d(k);
                    if let Some(rest) = k.strip_prefix("t.") {
                        let Some((oracle, field)) = rest.split_once('.') else { continue };
                        match field {
                            "comparisons" | "queries" => lines.push(format!("{p}.oracle.{field}:{v}|c|#{tags},oracle:{oracle}")),
                            _ => lines.push(format!("{p}.oracle.skips:{v}|c|#{tags},oracle:{oracle},reason:{field}")),
                        }
                    } else if let Some(variant) = k.strip_prefix("v.") {
                        lines.push(format!("{p}.variant.comparisons:{v}|c|#{tags},variant:{variant}"));
                    }
                }
                for (name, _) in ORACLES {
                    let o = |k: &str| d(&format!("o.{name}.{k}"));
                    for field in ["execs", "failures", "timeouts", "known", "ms"] {
                        lines.push(format!("{p}.oracle.{field}:{}|c|#{tags},oracle:{name}", o(field)));
                    }
                }
                for (i, w) in sv.workers.iter().enumerate() {
                    let stats: HashMap<String, u64> = read_pairs(&worker_dir(&dir, w.id).join("stats")).into_iter().filter_map(|(k, v)| Some((k, v.parse().ok()?))).collect();
                    let execs = stats.get("execs").copied().unwrap_or(0);
                    let wt = format!("{tags},worker:{}", w.id);
                    lines.push(format!("{p}.worker.execs_per_sec:{:.1}|g|#{wt}", execs.saturating_sub(last_execs[i]) as f64 / secs));
                    lines.push(format!("{p}.worker.restarts:{}|g|#{wt}", stats.get("restarts").copied().unwrap_or(0)));
                    if let Some(c) = &w.child {
                        lines.push(format!("{p}.worker.rss_mb:{}|g|#{wt}", rss_mb(c.id())));
                    }
                    if let Some(mb) = stats.get("backend").and_then(|b| backend_rss.get(&(*b as i32))) {
                        lines.push(format!("{p}.worker.backend_rss_mb:{mb}|g|#{wt}"));
                    }
                    last_execs[i] = execs;
                }
                for l in lines {
                    let _ = socket.send_to(l.as_bytes(), addr);
                }
            }
            sv.counters.save(&state_path);
            last = (Instant::now(), s);
        }
        if stopping && alive_n == 0 {
            sv.counters.save(&state_path);
            let out = Command::new(pg::bin_dir().join("pg_ctl")).arg("-D").arg(pg_dir.join("data")).args(["stop", "-m", "fast", "-w", "-t", "60"]).output();
            let ok = out.as_ref().is_ok_and(|o| o.status.success());
            let c = &sv.counters;
            sv.event(
                "run_stop",
                &format!("\"crashes\":{},\"hangs\":{},\"restarts\":{},\"memory_kills\":{},\"server_stopped\":{ok}", c.crashes, c.hangs, c.restarts, c.memory_kills),
                &format!("supervisor stops: all workers done, cluster stopped: {ok}"),
            );
            let _ = fs::remove_file(dir.join("supervisor.pid"));
            return;
        }
        sleep(Duration::from_secs(1));
    }
}

#[cfg(test)]
mod tests {
    use super::{crash_lines, terminated_pids};

    #[test]
    fn crashes_are_read_from_the_log() {
        let log = "2026-10-14 LOG:  server process (PID 4242) was terminated by signal 9: Killed\n2026 DETAIL:  Failed process was running: SELECT 1\n";
        assert!(crash_lines(log));
        assert_eq!(terminated_pids(log), vec![4242]);
        assert!(!crash_lines("LOG:  checkpoint starting: immediate force wait\n"));
        assert!(!crash_lines("LOG:  background worker \"parallel worker\" (PID 85447) exited with exit code 1\n"));
        assert!(crash_lines("LOG:  server process (PID 7) exited with exit code 2\n"));
    }
}
