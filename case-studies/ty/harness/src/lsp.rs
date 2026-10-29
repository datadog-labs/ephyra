// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The `lsp__` oracle (py/lsp.py): `ty server` over stdio, as an editor drives it, on the check
//! files of programs that passed the other oracles. Each is opened broken as a text document and
//! as a notebook (cells at top-level statements, non-ASCII comments, a negotiated position
//! encoding), restored with incremental edits, and compared with `ty check` and with itself:
//! `lsp__<cli|final|diff|closed|xpos>-<rule or request>__<text|nb>` (py/lsp.py's docstring), and
//! `crash__panic__*`, `crash__exit__tyserver`, `crash__hang__tyserver` when the server panics,
//! dies or stops answering.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

pub const LSP: &str = include_str!("../py/lsp.py");

/// Sessions run and requests sent (for stats.json).
pub static STATS: std::sync::Mutex<BTreeMap<String, u64>> = std::sync::Mutex::new(BTreeMap::new());

/// TYZ_LSP=0 turns the oracle off.
pub fn enabled() -> bool {
    std::env::var("TYZ_LSP").map(|v| v != "0").unwrap_or(true)
}

/// A finding: the check file it shows on (None: the whole session), signature, message.
pub type Found = (Option<String>, String, String);

/// The request a `lsp__xpos-<request>__nb` signature names (py/lsp.py compares it, and of the
/// throttled known-bad ones only it, when TYZ_LSP_FORCE names it); "none" for other signatures.
pub fn force_for(sig: &str) -> String {
    sig.strip_prefix("lsp__xpos-").and_then(|r| r.split("__").next()).unwrap_or("none").to_string()
}

/// Runs py/lsp.py on `files` (name -> check file text) with `seed`; `force`: TYZ_LSP_FORCE.
pub fn check(work: &Path, files: &BTreeMap<String, String>, seed: u64, force: Option<&str>, panic_sig: impl Fn(&str) -> String) -> Vec<Found> {
    let _ = fs::remove_dir_all(work);
    let ind = work.join("in");
    fs::create_dir_all(&ind).unwrap();
    fs::write(work.join("lsp.py"), LSP).unwrap();
    fs::write(ind.join("_rt.py"), "def _probe(k: int, v: object) -> None: ...\n").unwrap();
    for (n, t) in files {
        fs::write(ind.join(n), t).unwrap();
    }
    let mut cmd = Command::new(crate::py_bin());
    cmd.arg("-I").arg(work.join("lsp.py")).arg(crate::ty_bin()).arg(work.join("w")).arg(seed.to_string()).arg(&ind);
    cmd.args(files.keys());
    if let Some(f) = force {
        cmd.env("TYZ_LSP_FORCE", f);
    }
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::null()).spawn().expect("python");
    let mut so = child.stdout.take().unwrap();
    let ho = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = std::io::Read::read_to_string(&mut so, &mut s);
        s
    });
    let t0 = Instant::now();
    let timeout = Duration::from_secs(300);
    let finished = loop {
        if child.try_wait().unwrap().is_some() {
            break true;
        }
        if t0.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            break false;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let out = ho.join().unwrap_or_default();
    let mut found = vec![];
    let mut stats = STATS.lock().unwrap();
    *stats.entry("sessions".into()).or_default() += 1;
    for l in out.lines() {
        let Ok(v) = serde_json::from_str::<Value>(l) else { continue };
        if let Some(st) = v["stats"].as_object() {
            for (k, n) in st {
                *stats.entry(k.clone()).or_default() += n.as_u64().unwrap_or(0);
            }
            continue;
        }
        let prog = v["prog"].as_str().map(str::to_string);
        let kind = v["kind"].as_str().unwrap_or("");
        let msg = v["msg"].as_str().unwrap_or("").to_string();
        let sig = match kind {
            "panic" => panic_sig(&msg),
            "exit" => "crash__exit__tyserver".into(),
            "hang" => "crash__hang__tyserver".into(),
            "harness" => {
                // The harness's own failures (a restore that went wrong, a traceback): counted, not findings.
                *stats.entry("harness".into()).or_default() += 1;
                continue;
            }
            k => format!("lsp__{k}"),
        };
        found.push((prog, sig, format!("{msg} [encoding {}]", v["enc"].as_str().unwrap_or("?"))));
    }
    drop(stats);
    if !finished && found.is_empty() {
        found.push((None, "crash__hang__lsp".into(), format!("py/lsp.py ran past {timeout:?}")));
    }
    found
}

/// The finding's files: the check files of the session (one when it shows alone) and the seed.
pub fn finding_files(files: &BTreeMap<String, String>, seed: u64) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = files.iter().map(|(n, t)| (format!("lsp/in/{n}"), t.clone())).collect();
    v.push(("lsp/seed.txt".into(), seed.to_string()));
    v
}

/// `repro` for an lsp finding: 1 when the session on the saved files shows `sig` again.
pub fn repro(fdir: &Path, sig: &str, panic_sig: impl Fn(&str) -> String + Copy) -> i32 {
    let d = fdir.join("lsp");
    let seed: u64 = fs::read_to_string(d.join("seed.txt")).unwrap_or_default().trim().parse().unwrap_or(0);
    let mut files = BTreeMap::new();
    for e in fs::read_dir(d.join("in")).unwrap().flatten() {
        let n = e.file_name().to_string_lossy().to_string();
        if n.starts_with("c_") {
            files.insert(n, fs::read_to_string(e.path()).unwrap());
        }
    }
    let work: PathBuf = std::env::temp_dir().join(format!("tyz-lsp-repro-{}", std::process::id()));
    let force = force_for(sig);
    let mut hit = false;
    for _ in 0..2 {
        let f = check(&work, &files, seed, Some(&force), panic_sig);
        for (p, s, m) in &f {
            println!("{p:?} {s}: {m}");
        }
        if f.iter().any(|(_, s, _)| s == sig) {
            hit = true;
            break;
        }
    }
    let _ = fs::remove_dir_all(&work);
    hit as i32
}
