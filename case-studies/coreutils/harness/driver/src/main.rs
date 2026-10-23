// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! uudiff: differential search, uutils coreutils vs GNU coreutils 9.12, plus a
//! crash oracle. One case = one command line (util, argv, stdin, files in a
//! fresh working directory), run on both sides with the same clean
//! environment. See ../README.md.
//!
//! usage: uudiff <uu-bin-dir> <gnu-bin-dir> <seed> <seconds> <out-dir> [util,util,...]

mod gen;
mod known;

use ephyra::{Config, GenExt, Run, Source};
use gen::Case;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(5);
const FSIZE_LIMIT: u64 = 16 << 20;
const CAP_PER_SIG: usize = 3;

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub path: String,
    pub kind: char,
    pub mode: u32,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct Res {
    pub code: Option<i32>,
    pub signal: Option<i32>,
    pub timeout: bool,
    pub out: Vec<u8>,
    pub err: Vec<u8>,
    pub tree: Vec<Entry>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Verdict {
    Pass,
    Skip(&'static str),
    Masked(&'static str),
    Fail { oracle: &'static str, aspect: &'static str },
}

#[derive(Clone, Debug)]
struct Outcome {
    verdict: Verdict,
    uu: Res,
    gnu: Res,
}

impl Outcome {
    fn signature(&self, c: &Case) -> Option<String> {
        match &self.verdict {
            Verdict::Fail { oracle, aspect } => {
                let mut s = format!("{oracle}__{}_{aspect}", c.util);
                if let Some(area) = area(c) {
                    s.push_str("__");
                    s.push_str(area);
                }
                // sort -o: an output file that exists on one side only (created
                // before an input error) is a different code path from wrong
                // contents (ordering), which the stdout aspect also reaches.
                if c.util == "sort" && *aspect == "files" {
                    let paths = |r: &Res| r.tree.iter().map(|e| e.path.clone()).collect::<Vec<_>>();
                    s.push_str(if paths(&self.uu) != paths(&self.gnu) { "__output_created" } else { "__output_contents" });
                }
                Some(s)
            }
            _ => None,
        }
    }
}

/// Feature area for utilities whose options lead to unrelated code paths, so
/// that one bug in them does not hide the others behind one signature.
fn area(c: &Case) -> Option<&'static str> {
    let argv = c.argv();
    let has = |a: &str| argv.iter().any(|x| x == a);
    match c.util {
        // invalid-input handling, --format, output scaling (--to), unit
        // scaling, input suffixes, -z and field selection are separate paths
        "numfmt" => {
            let pre = |p: &str, except: &str| argv.iter().any(|a| a.starts_with(p) && a != except);
            Some(if pre("--invalid=", "--invalid=abort") {
                "invalid"
            } else if pre("--format=", "") {
                "format"
            } else if pre("--to=", "--to=none") {
                "to_scale"
            } else if pre("--to-unit=", "--to-unit=1") || pre("--from-unit=", "--from-unit=1") {
                "unit_scale"
            } else if pre("--from=", "--from=none") {
                "from_scale"
            } else if has("-z") {
                "zero_terminated"
            } else if pre("--field=", "") || has("-d") {
                "fields"
            } else {
                "plain"
            })
        }
        "cut" => Some(if has("--complement") {
            "complement"
        } else if argv.iter().any(|a| a.starts_with("--output-delimiter")) {
            "output_delimiter"
        } else if has("-s") {
            "only_delimited"
        } else if has("-z") {
            "zero_terminated"
        } else if has("-f") {
            "fields"
        } else {
            "bytes_chars"
        }),
        // --suppress-matched, and line numbers after a regex (where the two
        // read positions interact), are separate code paths in both tools
        "csplit" => {
            // operands: the input (possibly "-") and then the patterns
            let pats: Vec<&String> =
                c.args.iter().filter(|g| g.len() == 1 && (g[0] == "-" || !g[0].starts_with('-'))).map(|g| &g[0]).skip(1).collect();
            let regex_then_line = pats.iter().enumerate().any(|(i, p)| {
                p.parse::<u64>().is_ok() && pats[..i].iter().any(|q| q.starts_with('/') || q.starts_with('%'))
            });
            Some(if has("--suppress-matched") && regex_then_line {
                "suppress_matched_line_after_regex"
            } else if has("--suppress-matched") {
                "suppress_matched"
            } else if regex_then_line {
                "line_after_regex"
            } else {
                "patterns"
            })
        }
        // conversions go through separate formatters (quoting, %b escapes,
        // floats, integers, strings and format escapes)
        "printf" => {
            let f = argv.first().map(|s| s.as_bytes()).unwrap_or_default();
            let mut convs = Vec::new();
            let mut i = 0;
            while i < f.len() {
                if f[i] == b'%' {
                    let mut j = i + 1;
                    while j < f.len() && b"0123456789$-+ #.*'".contains(&f[j]) {
                        j += 1;
                    }
                    if let Some(&c) = f.get(j) {
                        convs.push(c);
                    }
                    i = j + 1;
                } else {
                    i += 1;
                }
            }
            let any = |s: &[u8]| convs.iter().any(|c| s.contains(c));
            Some(if any(b"q") {
                "quote"
            } else if any(b"b") {
                "escape_arg"
            } else if any(b"feEgGaA") {
                "float"
            } else if any(b"diouxXc") {
                "int"
            } else {
                "string"
            })
        }
        // suffix naming, the line/byte/chunk strategies and -u are parsed and
        // validated by separate code in both tools
        "split" => {
            let opt = |short: &str, long: &str| {
                argv.iter().any(|a| {
                    (a.starts_with(short) && !a.starts_with("--")) || (a.len() >= 4 && a.starts_with("--") && long.starts_with(a.split('=').next().unwrap()))
                })
            };
            // a line count with a unit suffix (-l 1k) is parsed by the line
            // strategy whatever suffix options come with it
            let lines_unit = argv.windows(2).any(|w| {
                (w[0] == "-l" || (w[0].len() >= 4 && "--lines".starts_with(w[0].as_str()))) && w[1].bytes().any(|b| !b.is_ascii_digit())
            }) || argv.iter().any(|a| {
                a.strip_prefix("--lines=").or_else(|| a.strip_prefix("-l")).is_some_and(|v| !v.is_empty() && v.bytes().any(|b| !b.is_ascii_digit()))
            });
            Some(if opt("-u", "--unbuffered") {
                "unbuffered"
            } else if lines_unit {
                "lines"
            // plain -d/-x only change the alphabet; counting them would file
            // strategy bugs (-l 1k -d) under suffix
            } else if opt("-a", "--suffix-length")
                || argv.iter().any(|a| a.starts_with("--numeric-suffixes=") || a.starts_with("--hex-suffixes="))
                || opt("\0", "--additional-suffix")
            {
                "suffix"
            } else if opt("-n", "--number") {
                "number"
            } else if opt("-b", "--bytes") || opt("-C", "--line-bytes") {
                "bytes"
            } else if opt("-l", "--lines") {
                "lines"
            } else {
                "other"
            })
        }
        // utilities added in steering round 1: their main option families
        "date" => {
            let d = argv.iter().find_map(|a| a.strip_prefix("--date=")).map(String::from).or_else(|| {
                argv.windows(2).find(|w| w[0] == "-d").map(|w| w[1].clone())
            });
            let d = d.unwrap_or_default();
            let rel = ["day", "week", "month", "year", "hour", "minute", "sec", "fortnight", "tomorrow", "yesterday", "today", "next", "last", "this", "ago", "sunday", "monday", "thursday", "friday"];
            Some(if has("-f") {
                "file"
            } else if argv.iter().any(|a| a.starts_with("-I") || a == "-R" || a.starts_with("--rfc-3339")) {
                "rfc"
            } else if d.starts_with('@') {
                "epoch"
            } else if rel.iter().any(|r| d.contains(r)) {
                "relative"
            } else if d.contains("TZ=")
                || d.split(' ').skip(1).any(|w| {
                    matches!(w, "UTC" | "Z" | "GMT" | "EST" | "PDT" | "CET" | "UTC+3" | "J") || w.starts_with(['+', '-'])
                })
            {
                "zone"
            } else {
                "format"
            })
        }
        "pr" => Some(if has("-m") {
            "merge"
        } else if argv.iter().any(|a| matches!(a.as_str(), "-2" | "-3") || a.starts_with("--columns")) {
            "columns"
        } else if argv.iter().any(|a| a.starts_with('+') || a.starts_with("--pages")) {
            "pages"
        } else if has("-w") || has("-W") || has("-l") {
            "page_size"
        } else if argv.iter().any(|a| a.starts_with("-n") || a.starts_with("-e") || a.starts_with("-i") || a == "-N" || a.starts_with("--first-line")) {
            "numbering_tabs"
        } else {
            "plain"
        }),
        "ptx" => Some(if has("-G") { "traditional" } else { "gnu" }),
        "env" => Some(if argv.iter().any(|a| a == "-S" || a.starts_with("--split-string")) { "split_string" } else { "plain" }),
        "chmod" | "mkdir" | "install" => Some(if argv.iter().any(|a| a.starts_with("--reference")) {
            "reference"
        } else if has("-v") || has("-c") {
            "verbose"
        } else if c.util == "install" && (has("-b") || argv.iter().any(|a| a.starts_with("--backup") || a == "-S" || a.starts_with("--suffix"))) {
            "backup"
        } else {
            "mode"
        }),
        "cp" | "mv" | "ln" => Some(if has("-b") || argv.iter().any(|a| a.starts_with("--backup") || a == "-S" || a.starts_with("--suffix")) {
            "backup"
        } else if has("-t") || has("-T") {
            "target"
        } else if argv.iter().any(|a| a.starts_with("-r") || a.starts_with("-R") || a.starts_with("-a")) {
            "recursive"
        } else {
            "plain"
        }),
        "ls" => Some(if argv.iter().any(|a| matches!(a.as_str(), "-l" | "-g" | "-o" | "-n" | "--format=long" | "--format=verbose")) {
            "long"
        } else if argv.iter().any(|a| a.starts_with("--quoting-style") || matches!(a.as_str(), "-Q" | "-b" | "-N" | "-q" | "--literal" | "--show-control-chars")) {
            "quoting"
        } else if has("-R") {
            "recursive"
        } else {
            "layout"
        }),
        "du" => Some(if argv.iter().any(|a| a == "-B" || matches!(a.as_str(), "-h" | "--si" | "-k" | "-m")) { "block_size" } else { "traversal" }),
        _ => None,
    }
}

// ---------------------------------------------------------------- running

fn prepare(dir: &Path, c: &Case) -> std::io::Result<()> {
    if dir.exists() && fs::remove_dir_all(dir).is_err() {
        let _ = Command::new("chmod").args(["-R", "u+rwx"]).arg(dir).status();
        fs::remove_dir_all(dir)?;
    }
    fs::create_dir_all(dir.join("w"))?;
    for (name, data) in &c.files {
        if let Some(d) = name.strip_suffix('/') {
            fs::create_dir_all(dir.join("w").join(d))?;
        } else if let Some(l) = name.strip_suffix('@') {
            // symlink: the data is its target
            std::os::unix::fs::symlink(String::from_utf8_lossy(data).as_ref(), dir.join("w").join(l))?;
        } else {
            fs::write(dir.join("w").join(name), data)?;
        }
    }
    fs::write(dir.join("in"), &c.stdin)?;
    Ok(())
}

/// LLVM_PROFILE_FILE for the target processes. Each instrumented process writes
/// ~5 MB of counters, so a per-process pattern (%p) fills the disk within minutes
/// (17 GB in 30 s measured). Same directory, but with LLVM's online merging (%m,
/// or %4m when the pattern has none) instead of %p; the counts are the same.
fn profile_file() -> Option<String> {
    static P: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    P.get_or_init(|| {
        let v = std::env::var("LLVM_PROFILE_FILE").ok()?;
        if !v.contains("%p") {
            return Some(v);
        }
        let has_merge = v.match_indices('%').any(|(i, _)| {
            v[i + 1..].trim_start_matches(|c: char| c.is_ascii_digit()).starts_with('m')
        });
        Some(if has_merge { v.replace("%p", "") } else { v.replace("%p", "%4m") })
    })
    .clone()
}

fn spawn(bin: &Path, c: &Case, dir: &Path) -> std::io::Result<Child> {
    let mut cmd = Command::new(bin.join(c.util));
    cmd.arg0(c.util)
        .args(c.argv())
        .env_clear()
        .env("PATH", bin)
        .env("LC_ALL", "C")
        .env("TZ", "UTC0")
        // coverage builds: the controller sets LLVM_PROFILE_FILE; keep it through env_clear
        .envs(profile_file().map(|v| ("LLVM_PROFILE_FILE", v)))
        .current_dir(dir.join("w"))
        .stdin(fs::File::open(dir.join("in"))?)
        .stdout(fs::File::create(dir.join("out"))?)
        .stderr(fs::File::create(dir.join("err"))?)
        .process_group(0);
    unsafe {
        cmd.pre_exec(|| {
            let lim = libc::rlimit { rlim_cur: FSIZE_LIMIT, rlim_max: FSIZE_LIMIT };
            libc::setrlimit(libc::RLIMIT_FSIZE, &lim);
            libc::signal(libc::SIGPIPE, libc::SIG_DFL);
            Ok(())
        });
    }
    cmd.spawn()
}

fn walk(root: &Path, rel: &str, out: &mut Vec<Entry>) {
    let Ok(rd) = fs::read_dir(root.join(rel)) else { return };
    let mut names: Vec<String> = rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    for n in names {
        let p = if rel.is_empty() { n.clone() } else { format!("{rel}/{n}") };
        let full = root.join(&p);
        let Ok(md) = fs::symlink_metadata(&full) else { continue };
        let ft = md.file_type();
        let mode = md.mode() & 0o7777;
        if ft.is_symlink() {
            let t = fs::read_link(&full).map(|t| t.to_string_lossy().into_owned().into_bytes()).unwrap_or_default();
            out.push(Entry { path: p, kind: 'l', mode: 0, data: t });
        } else if ft.is_dir() {
            out.push(Entry { path: p.clone(), kind: 'd', mode, data: vec![] });
            walk(root, &p, out);
        } else if ft.is_file() {
            out.push(Entry { path: p, kind: 'f', mode, data: fs::read(&full).unwrap_or_default() });
        } else {
            out.push(Entry { path: p, kind: 'o', mode, data: vec![] });
        }
    }
}

fn collect(dir: &Path, st: Option<std::process::ExitStatus>) -> Res {
    let mut tree = Vec::new();
    walk(&dir.join("w"), "", &mut tree);
    Res {
        code: st.and_then(|s| s.code()),
        signal: st.and_then(|s| s.signal()),
        timeout: st.is_none(),
        out: fs::read(dir.join("out")).unwrap_or_default(),
        err: fs::read(dir.join("err")).unwrap_or_default(),
        tree,
    }
}

/// Run both sides at once; returns (uu, gnu).
fn run_pair(uu_bin: &Path, gnu_bin: &Path, c: &Case, scratch: &Path) -> (Res, Res) {
    let du = scratch.join("uu");
    let dg = scratch.join("gnu");
    prepare(&du, c).expect("prepare uu");
    prepare(&dg, c).expect("prepare gnu");
    let mut kids = [spawn(uu_bin, c, &du).expect("spawn uu"), spawn(gnu_bin, c, &dg).expect("spawn gnu")];
    let mut st: [Option<Option<std::process::ExitStatus>>; 2] = [None, None];
    let start = Instant::now();
    let mut nap = 50;
    while st.iter().any(|s| s.is_none()) {
        for i in 0..2 {
            if st[i].is_none() {
                if let Ok(Some(s)) = kids[i].try_wait() {
                    st[i] = Some(Some(s));
                }
            }
        }
        if st.iter().all(|s| s.is_some()) {
            break;
        }
        if start.elapsed() > TIMEOUT {
            for i in 0..2 {
                if st[i].is_none() {
                    unsafe { libc::kill(-(kids[i].id() as i32), libc::SIGKILL) };
                    let _ = kids[i].wait();
                    st[i] = Some(None);
                }
            }
            break;
        }
        std::thread::sleep(Duration::from_micros(nap));
        nap = (nap * 2).min(2000);
    }
    (collect(&du, st[0].unwrap()), collect(&dg, st[1].unwrap()))
}

fn contains(h: &[u8], n: &[u8]) -> bool {
    h.windows(n.len()).any(|w| w == n)
}

fn judge(c: &Case, uu: &Res, gnu: &Res) -> Verdict {
    if contains(&uu.err, b"panicked at") || uu.code == Some(101) {
        return Verdict::Fail { oracle: "crash", aspect: "panic" };
    }
    if uu.signal.is_some() && uu.signal != gnu.signal {
        return Verdict::Fail { oracle: "crash", aspect: "signal" };
    }
    if gnu.timeout {
        return Verdict::Skip("gnu_timeout");
    }
    if uu.timeout {
        return Verdict::Fail { oracle: "crash", aspect: "timeout" };
    }
    if gnu.signal.is_some() {
        return Verdict::Skip("gnu_signal");
    }
    if let Some(k) = known::known(c, uu, gnu) {
        return Verdict::Masked(k);
    }
    if uu.code != gnu.code {
        return Verdict::Fail { oracle: "diff", aspect: "exit" };
    }
    if uu.out != gnu.out {
        return Verdict::Fail { oracle: "diff", aspect: "stdout" };
    }
    if uu.tree != gnu.tree {
        return Verdict::Fail { oracle: "diff", aspect: "files" };
    }
    if uu.err.is_empty() != gnu.err.is_empty() {
        return Verdict::Fail { oracle: "diff", aspect: "stderr" };
    }
    Verdict::Pass
}

// ---------------------------------------------------------------- findings

const REPRO: &str = include_str!("repro.sh");

fn write_case(dir: &Path, c: &Case) -> std::io::Result<()> {
    let files = dir.join("case/files");
    fs::create_dir_all(&files)?;
    for (name, data) in &c.files {
        if let Some(d) = name.strip_suffix('/') {
            fs::create_dir_all(files.join(d))?;
        } else if let Some(l) = name.strip_suffix('@') {
            std::os::unix::fs::symlink(String::from_utf8_lossy(data).as_ref(), files.join(l))?;
        } else {
            fs::write(files.join(name), data)?;
        }
    }
    fs::write(dir.join("case/stdin"), &c.stdin)?;
    let mut a = Vec::new();
    for s in c.argv() {
        a.extend_from_slice(s.as_bytes());
        a.push(0);
    }
    fs::write(dir.join("case/args"), a)?;
    Ok(())
}

fn shell_quote(s: &str) -> String {
    if !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_=.,/:+%".contains(&b)) {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

fn show(b: &[u8], n: usize) -> String {
    let s = String::from_utf8_lossy(&b[..b.len().min(n)]).into_owned();
    format!("{s:?}{}", if b.len() > n { "..." } else { "" })
}

struct Search {
    uu_bin: PathBuf,
    gnu_bin: PathBuf,
    build: PathBuf,
    out: PathBuf,
    scratch: PathBuf,
    cases: u64,
    failures: u64,
    masked: BTreeMap<String, u64>,
    skipped: BTreeMap<String, u64>,
    fail_by_sig: BTreeMap<String, u64>,
    by_util: BTreeMap<String, u64>,
    findings: BTreeMap<String, usize>,
    repro_failed: u64,
    flaky: u64,
    started: Instant,
}

impl Search {
    fn stats(&self) {
        let secs = self.started.elapsed().as_secs_f64();
        let j = serde_json::json!({
            "cases": self.cases,
            "failures": self.failures,
            "elapsed_s": (secs * 10.0).round() / 10.0,
            "cases_per_s": ((self.cases as f64 / secs.max(0.001)) * 10.0).round() / 10.0,
            "masked_known": self.masked,
            "skipped": self.skipped,
            "failures_by_signature": self.fail_by_sig,
            "cases_by_util": self.by_util,
            "findings_written": self.findings,
            "repro_unconfirmed": self.repro_failed,
            "flaky": self.flaky,
        });
        let tmp = self.out.join("stats.json.tmp");
        if fs::write(&tmp, serde_json::to_vec_pretty(&j).unwrap()).is_ok() {
            let _ = fs::rename(&tmp, self.out.join("stats.json"));
        }
    }

    fn write_finding(&mut self, sig: &str, c: &Case, o: &Outcome, seed: &str) {
        let n = *self.findings.get(sig).unwrap_or(&0);
        let dir = self.out.join("findings").join(sig).join(n.to_string());
        let _ = fs::remove_dir_all(&dir);
        if write_case(&dir, c).is_err() {
            return;
        }
        let Verdict::Fail { oracle, aspect } = o.verdict.clone() else { return };
        let script = REPRO.replace("@UTIL@", c.util).replace("@ASPECT@", aspect);
        let rp = dir.join("repro.sh");
        fs::write(&rp, script).unwrap();
        fs::set_permissions(&rp, fs::Permissions::from_mode(0o755)).unwrap();
        // expected/ = the reference's behaviour, recorded by the repro script itself.
        let rec = Command::new("bash").arg(&rp).arg("--record").arg(&self.gnu_bin).current_dir(&dir).status();
        let rep = Command::new("bash").arg(&rp).arg(&self.build).current_dir(&dir).stdout(Stdio::null()).stderr(Stdio::null()).status();
        if !matches!(rec, Ok(s) if s.success()) || !matches!(rep, Ok(s) if s.code() == Some(1)) {
            self.repro_failed += 1;
            let _ = fs::remove_dir_all(&dir);
            return;
        }
        let obs = dir.join("observed");
        let _ = fs::create_dir_all(&obs);
        for (side, r) in [("uu", &o.uu), ("gnu", &o.gnu)] {
            let _ = fs::write(obs.join(format!("{side}.stdout")), &r.out);
            let _ = fs::write(obs.join(format!("{side}.stderr")), &r.err);
            let _ = fs::write(obs.join(format!("{side}.status")), format!("code={:?} signal={:?} timeout={}\n", r.code, r.signal, r.timeout));
        }
        let cmd = std::iter::once(c.util.to_string()).chain(c.argv().iter().map(|s| shell_quote(s))).collect::<Vec<_>>().join(" ");
        let msg = format!(
            "{cmd}  <stdin {}  | uu: code={:?} out={} err={} | gnu: code={:?} out={} err={}",
            show(&c.stdin, 60),
            o.uu.code,
            show(&o.uu.out, 80),
            show(&o.uu.err, 120),
            o.gnu.code,
            show(&o.gnu.out, 80),
            show(&o.gnu.err, 120)
        );
        let j = serde_json::json!({"signature": sig, "oracle": oracle, "message": msg, "seed": seed, "util": c.util, "aspect": aspect, "argv": c.argv()});
        fs::write(dir.join("finding.json"), serde_json::to_vec_pretty(&j).unwrap()).unwrap();
        self.findings.insert(sig.to_string(), n + 1);
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 6 {
        eprintln!("usage: uudiff <uu-bin-dir> <gnu-bin-dir> <seed> <seconds> <out-dir> [utils]");
        std::process::exit(2);
    }
    let uu_bin = fs::canonicalize(&a[1]).expect("uu bin dir");
    let gnu_bin = fs::canonicalize(&a[2]).expect("gnu bin dir");
    let seed: u64 = a[3].parse().expect("seed");
    let secs: u64 = a[4].parse().expect("seconds");
    let out = PathBuf::from(&a[5]);
    fs::create_dir_all(out.join("findings")).unwrap();
    let out = fs::canonicalize(out).unwrap();
    let only: Option<Vec<String>> = a.get(6).map(|s| s.split(',').map(String::from).collect());
    let scratch = std::env::temp_dir().join(format!("uudiff-{}-{seed}", std::process::id()));
    fs::create_dir_all(&scratch).unwrap();
    let build = uu_bin.parent().unwrap().to_path_buf();
    let mut s = Search {
        uu_bin,
        gnu_bin,
        build,
        out,
        scratch: scratch.clone(),
        cases: 0,
        failures: 0,
        masked: BTreeMap::new(),
        skipped: BTreeMap::new(),
        fail_by_sig: BTreeMap::new(),
        by_util: BTreeMap::new(),
        findings: BTreeMap::new(),
        repro_failed: 0,
        flaky: 0,
        started: Instant::now(),
    };
    s.stats();
    let deadline = s.started + Duration::from_secs(secs);
    let mut last_stats = Instant::now();
    let mut round: u64 = 0;
    std::panic::set_hook(Box::new(|_| {})); // property panics are expected; ephyra records them
    while Instant::now() < deadline {
        let run_seed = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(round);
        round += 1;
        let mut cfg = Config::default().tests(1 << 40).seed(run_seed).no_regressions();
        cfg.shrink.max_steps = 300;
        cfg.shrink.timeout_ms = Some(20_000);
        let only2 = only.clone();
        let generator = (move |g: &mut Source| gen::case(g, only2.as_deref())).with_shrink(gen::shrink_case);
        let last: Rc<RefCell<Option<Outcome>>> = Rc::new(RefCell::new(None));
        let last2 = last.clone();
        let (ub, gb, sc) = (s.uu_bin.clone(), s.gnu_bin.clone(), s.scratch.clone());
        let property = move |c: Case| {
            let (uu, gnu) = run_pair(&ub, &gb, &c, &sc);
            let verdict = judge(&c, &uu, &gnu);
            *last2.borrow_mut() = Some(Outcome { verdict: verdict.clone(), uu, gnu });
            // One panic site per aspect, so shrinking keeps the failure kind.
            match verdict {
                Verdict::Fail { aspect: "panic", .. } => panic!("crash: panic"),
                Verdict::Fail { aspect: "signal", .. } => panic!("crash: signal"),
                Verdict::Fail { aspect: "timeout", .. } => panic!("crash: timeout"),
                Verdict::Fail { aspect: "exit", .. } => panic!("diff: exit"),
                Verdict::Fail { aspect: "stdout", .. } => panic!("diff: stdout"),
                Verdict::Fail { aspect: "files", .. } => panic!("diff: files"),
                Verdict::Fail { .. } => panic!("diff: stderr"),
                _ => {}
            }
        };
        let mut run = Run::new(&cfg, generator, property);
        while Instant::now() < deadline && run.wants_more() {
            let Some(input) = run.draw() else { continue };
            let tested = run.test(&input);
            s.cases += 1;
            *s.by_util.entry(input.value.util.to_string()).or_default() += 1;
            let o = last.borrow().clone().unwrap();
            match &o.verdict {
                Verdict::Masked(k) => *s.masked.entry(k.to_string()).or_default() += 1,
                Verdict::Skip(k) => *s.skipped.entry(k.to_string()).or_default() += 1,
                _ => {}
            }
            if tested.failed() {
                s.failures += 1;
                let case = input.value.clone();
                let sig = o.signature(&case).unwrap_or_else(|| "unknown__unknown".into());
                *s.fail_by_sig.entry(sig.clone()).or_default() += 1;
                if s.findings.get(&sig).copied().unwrap_or(0) < CAP_PER_SIG {
                    let shrunk = run.shrink().cloned();
                    let mut best = (case.clone(), o.clone());
                    if let Some(sh) = shrunk {
                        let (uu, gnu) = run_pair(&s.uu_bin, &s.gnu_bin, &sh, &s.scratch);
                        let v = judge(&sh, &uu, &gnu);
                        let o2 = Outcome { verdict: v, uu, gnu };
                        if o2.signature(&sh).as_deref() == Some(&sig) {
                            best = (sh, o2);
                        } else {
                            s.flaky += 1;
                        }
                    }
                    let (uu, gnu) = run_pair(&s.uu_bin, &s.gnu_bin, &best.0, &s.scratch);
                    let v = judge(&best.0, &uu, &gnu);
                    let o3 = Outcome { verdict: v, uu, gnu };
                    if o3.signature(&best.0).as_deref() == Some(&sig) {
                        s.write_finding(&sig, &best.0, &o3, &format!("{seed}:{run_seed}"));
                    } else {
                        s.flaky += 1;
                    }
                }
                break; // a Run stops at its first failure; start a fresh one
            }
            if last_stats.elapsed() > Duration::from_secs(2) {
                s.stats();
                last_stats = Instant::now();
            }
        }
        s.stats();
    }
    s.stats();
    let _ = fs::remove_dir_all(&scratch);
    let _ = std::io::stdout().flush();
}
