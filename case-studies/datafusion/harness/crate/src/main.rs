// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! dfz: config-toggle differential + crash oracle for DataFusion.
//!
//!   dfz search <seed> <seconds> <out-dir>   supervisor: generates cases, runs them in a worker child
//!   dfz worker                              reads one JSON case per line on stdin, prints one verdict line
//!   dfz repro <case.json> <signature>       exit 1 if the signature reproduces, 0 if not, 2 if undecidable
//!   dfz gen <seed> <count>                  print generated queries (to look at the generator)
//!   dfz run <case.json>                     run a case in-process and print the verdict

mod case;
mod exec;
mod casegen;

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use case::{Case, Verdict};

const CASE_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_PER_SIG: usize = 3;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .thread_stack_size(32 << 20)
        .enable_all()
        .build()
        .unwrap()
}

fn worker() {
    exec::install_panic_hook();
    let rt = runtime();
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let v = match serde_json::from_str::<Case>(&line) {
            Ok(case) => exec::judge(&rt, &case),
            Err(e) => Verdict { skip: Some("bad_case".into()), msg: e.to_string(), ..Default::default() },
        };
        writeln!(out, "{}", serde_json::to_string(&v).unwrap()).unwrap();
        out.flush().unwrap();
    }
}

/// A worker child process with a reader thread, so replies can be awaited with a timeout.
struct Worker {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<Option<String>>,
    stderr_path: PathBuf,
}

enum Reply {
    Verdict(Verdict),
    Hang,
    Died(String),
}

impl Worker {
    fn spawn(stderr_path: &Path) -> Worker {
        let exe = std::env::current_exe().unwrap();
        let errf = std::fs::File::create(stderr_path).unwrap();
        let mut child = Command::new(exe)
            .arg("worker")
            .env("RUST_BACKTRACE", "0")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(errf))
            .spawn()
            .expect("spawn worker");
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let mut r = BufReader::new(stdout);
            loop {
                let mut s = String::new();
                match r.read_line(&mut s) {
                    Ok(0) | Err(_) => {
                        let _ = tx.send(None);
                        break;
                    }
                    Ok(_) => {
                        if tx.send(Some(s)).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        Worker { child, stdin, rx, stderr_path: stderr_path.to_path_buf() }
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    /// Close stdin so the worker exits normally (and writes its coverage profile);
    /// kill it if it does not exit within a few seconds.
    fn shutdown(mut self) {
        drop(self.stdin);
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    fn ask(&mut self, case: &Case) -> Reply {
        let line = serde_json::to_string(case).unwrap();
        if writeln!(self.stdin, "{line}").and_then(|_| self.stdin.flush()).is_err() {
            return self.died();
        }
        match self.rx.recv_timeout(CASE_TIMEOUT) {
            Ok(Some(s)) => match serde_json::from_str::<Verdict>(&s) {
                Ok(v) => Reply::Verdict(v),
                Err(e) => Reply::Died(format!("bad reply {e}: {s}")),
            },
            Ok(None) | Err(RecvTimeoutError::Disconnected) => self.died(),
            Err(RecvTimeoutError::Timeout) => {
                self.kill();
                Reply::Hang
            }
        }
    }

    fn died(&mut self) -> Reply {
        let status = self.child.wait().map(|s| format!("{s}")).unwrap_or_default();
        let err = std::fs::read_to_string(&self.stderr_path).unwrap_or_default();
        let tail: String = err.lines().rev().take(5).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join(" / ");
        Reply::Died(format!("worker exited ({status}): {tail}"))
    }
}

/// Signature and message for a reply, or None when the case passed / was skipped.
fn classify(r: &Reply) -> Option<(String, String)> {
    match r {
        Reply::Verdict(v) => v.sig.clone().map(|s| (s, v.msg.clone())),
        Reply::Hang => Some(("crash__hang".into(), format!("no verdict within {}s", CASE_TIMEOUT.as_secs()))),
        Reply::Died(m) => {
            let kind = if m.contains("overflowed its stack") { "crash__stack_overflow" } else { "crash__abort" };
            Some((kind.into(), m.clone()))
        }
    }
}

struct Searcher {
    worker: Worker,
    stderr_path: PathBuf,
}

impl Searcher {
    fn check(&mut self, case: &Case) -> Reply {
        let r = self.worker.ask(case);
        if !matches!(r, Reply::Verdict(_)) {
            self.worker.kill();
            self.worker = Worker::spawn(&self.stderr_path);
        }
        r
    }

    fn same(&mut self, case: &Case, sig: &str) -> bool {
        classify(&self.check(case)).is_some_and(|(s, _)| s == sig)
    }

    /// Greedy reduction that keeps the signature: drop other variants, then rows.
    fn reduce(&mut self, mut case: Case, sig: &str) -> Case {
        if sig.starts_with("toggle__") && case.variants.len() > 1 {
            let name = sig.split_once("_rows_").or(sig.split_once("_error_")).map(|(_, n)| n.to_string());
            if let Some(n) = name {
                let mut c = case.clone();
                c.variants.retain(|v| v.name == n);
                if c.variants.len() == 1 && self.same(&c, sig) {
                    case = c;
                }
            }
        }
        if matches!(sig, "crash__hang") {
            return case; // each check costs the full timeout
        }
        let mut budget = 200;
        for ti in 0..case.tables.len() {
            let mut i = 0;
            while i < case.tables[ti].rows.len() && budget > 0 {
                budget -= 1;
                let mut c = case.clone();
                remove_row(&mut c.tables[ti], i);
                if self.same(&c, sig) {
                    case = c;
                } else {
                    i += 1;
                }
            }
        }
        case
    }
}

fn remove_row(t: &mut case::Table, i: usize) {
    t.rows.remove(i);
    let mut at = 0;
    for p in t.parts.iter_mut() {
        if i < at + *p {
            *p -= 1;
            break;
        }
        at += *p;
    }
}

fn write_finding(out: &Path, sig: &str, n: usize, case: &Case, msg: &str, seed: u64) {
    let dir = out.join("findings").join(sig).join(n.to_string());
    std::fs::create_dir_all(&dir).unwrap();
    let oracle = sig.split("__").next().unwrap_or("dfz");
    let finding = serde_json::json!({"signature": sig, "oracle": oracle, "message": msg, "seed": seed});
    std::fs::write(dir.join("finding.json"), serde_json::to_string_pretty(&finding).unwrap()).unwrap();
    std::fs::write(dir.join("case.json"), serde_json::to_string_pretty(case).unwrap()).unwrap();
    std::fs::write(dir.join("case.sql"), case::to_sql_script(case)).unwrap();
    let repro = format!(
        "#!/usr/bin/env bash\n# usage: repro.sh <build-dir>; exit 1 = reproduces, 0 = does not, 2 = cannot decide\n\
         cd \"$(dirname \"$0\")\" || exit 2\n[ -x \"$1/bin/dfz\" ] || exit 2\nexec \"$1/bin/dfz\" repro case.json '{sig}'\n"
    );
    let p = dir.join("repro.sh");
    std::fs::write(&p, repro).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn search(seed: u64, secs: u64, out: &Path) {
    std::fs::create_dir_all(out.join("findings")).unwrap();
    let stderr_path = out.join("worker.stderr");
    let mut s = Searcher { worker: Worker::spawn(&stderr_path), stderr_path };
    // stop a little early so the worker can exit cleanly (and flush its coverage profile)
    let deadline = Instant::now() + Duration::from_secs(secs.saturating_sub((secs / 20).min(5)));
    let start = Instant::now();
    let mut cases = 0u64;
    let mut failures = 0u64;
    let mut counters: BTreeMap<String, u64> = BTreeMap::new();
    let mut per_sig: BTreeMap<String, usize> = BTreeMap::new();
    let mut base_errs: BTreeMap<String, (u64, String)> = BTreeMap::new();
    let mut last_stats = Instant::now();
    let write_stats = |cases: u64, failures: u64, counters: &BTreeMap<String, u64>, per_sig: &BTreeMap<String, usize>| {
        let mut j = serde_json::json!({"cases": cases, "failures": failures,
            "elapsed_s": start.elapsed().as_secs_f64(),
            "cases_per_s": cases as f64 / start.elapsed().as_secs_f64().max(1e-9)});
        for (k, v) in counters {
            j[k] = serde_json::json!(v);
        }
        j["signatures"] = serde_json::json!(per_sig);
        let tmp = out.join("stats.json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(&j).unwrap()).unwrap();
        std::fs::rename(&tmp, out.join("stats.json")).unwrap();
    };
    write_stats(0, 0, &counters, &per_sig);
    let mut i = 0u64;
    while Instant::now() < deadline {
        let case_seed = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(i);
        i += 1;
        let case = casegen::case(case_seed);
        let r = s.check(&case);
        cases += 1;
        if let Reply::Verdict(v) = &r {
            if let Some(k) = &v.skip {
                *counters.entry(format!("skip_{k}")).or_default() += 1;
            }
            for n in &v.notes {
                *counters.entry(format!("note_{n}")).or_default() += 1;
            }
            if let Some(e) = &v.base_err {
                *counters.entry("base_error".into()).or_default() += 1;
                let key: String = e.chars().take(90).collect();
                base_errs.entry(key).or_insert((0, case.query.clone())).0 += 1;
            }
        }
        if let Some((sig, msg)) = classify(&r) {
            failures += 1;
            let n = per_sig.entry(sig.clone()).or_default();
            if *n < MAX_PER_SIG {
                let idx = *n;
                *n += 1;
                let reduced = s.reduce(case.clone(), &sig);
                write_finding(out, &sig, idx, &reduced, &msg, case_seed);
            } else {
                *n += 0;
            }
            *counters.entry(format!("fail_{sig}")).or_default() += 1;
        }
        if last_stats.elapsed() > Duration::from_secs(5) {
            write_stats(cases, failures, &counters, &per_sig);
            last_stats = Instant::now();
        }
    }
    write_stats(cases, failures, &counters, &per_sig);
    let mut errs: Vec<(u64, String, String)> = base_errs.into_iter().map(|(k, (n, q))| (n, k, q)).collect();
    errs.sort_by(|a, b| b.cmp(a));
    let text: Vec<String> = errs.iter().take(60).map(|(n, e, q)| format!("{n}\t{e}\n\t{q}")).collect();
    let _ = std::fs::write(out.join("base_errors.txt"), text.join("\n"));
    s.worker.shutdown();
}

fn repro(path: &str, sig: &str) -> i32 {
    let Ok(text) = std::fs::read_to_string(path) else { return 2 };
    let Ok(case) = serde_json::from_str::<Case>(&text) else { return 2 };
    let tmp = std::env::temp_dir().join(format!("dfz-repro-{}.stderr", std::process::id()));
    let mut s = Searcher { worker: Worker::spawn(&tmp), stderr_path: tmp.clone() };
    // multi-partition execution is scheduled nondeterministically: allow a few attempts
    let attempts = if sig == "crash__hang" { 1 } else { 5 };
    let mut code = 0;
    for _ in 0..attempts {
        let r = s.check(&case);
        if let Reply::Verdict(v) = &r {
            if v.skip.as_deref() == Some("bad_case") {
                code = 2;
                break;
            }
        }
        if let Some((got, msg)) = classify(&r) {
            // a coarse signature from an older harness also matches its refinements
            if got == sig || got.starts_with(&format!("{sig}__")) {
                eprintln!("reproduced {sig}: {msg}");
                code = 1;
                break;
            }
            eprintln!("different failure {got}: {msg}");
        }
    }
    s.worker.shutdown();
    let _ = std::fs::remove_file(tmp);
    if code == 0 {
        eprintln!("did not reproduce {sig}");
    }
    code
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("worker") => worker(),
        Some("search") => search(args[2].parse().unwrap(), args[3].parse().unwrap(), Path::new(&args[4])),
        Some("repro") => std::process::exit(repro(&args[2], &args[3])),
        Some("gen") => {
            let seed: u64 = args[2].parse().unwrap();
            let n: u64 = args.get(3).map(|s| s.parse().unwrap()).unwrap_or(10);
            for i in 0..n {
                let c = casegen::case(seed.wrapping_add(i));
                if args.get(4).is_some_and(|a| a == "json") {
                    println!("{}", serde_json::to_string(&c).unwrap());
                } else {
                    println!("{};", c.query);
                }
            }
        }
        Some("run") => {
            exec::install_panic_hook();
            let case: Case = serde_json::from_str(&std::fs::read_to_string(&args[2]).unwrap()).unwrap();
            let v = exec::judge(&runtime(), &case);
            println!("{}", serde_json::to_string_pretty(&v).unwrap());
        }
        Some("plan") => {
            let case: Case = serde_json::from_str(&std::fs::read_to_string(&args[2]).unwrap()).unwrap();
            for (name, p) in exec::plans(&runtime(), &case) {
                println!("== {name}\n{}", p.unwrap_or_else(|| "<no plan>".into()));
            }
        }
        _ => {
            eprintln!("usage: dfz search|worker|repro|gen|run|plan ...");
            std::process::exit(2);
        }
    }
}
