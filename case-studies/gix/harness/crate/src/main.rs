// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! gixfuzz: differential revspec oracle (gix `rev_parse_single` vs git `rev-parse --verify`)
//! plus a crash oracle (panic / timeout / abort inside gix).
//!
//! Modes:
//!   gixfuzz search <seed> <seconds> <out-dir>   one search process (run.sh supervises it)
//!   gixfuzz check <case.json>                   exit 1 = reproduces, 0 = does not, 2 = cannot decide
//!   gixfuzz eval <case.json>                    child of `check`: prints the signature or PASS
//!   gixfuzz record-abort <out-dir>              turn <out-dir>/current.json into a crash__abort finding
//!   gixfuzz sample <seed> <n>                   print rendered scenarios (for looking at the generator)

mod gen;
mod graph;
mod merge;
mod treediff;
mod wt;

use ephyra::{Config, Run, TestResult};
use gen::Scenario;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// A concrete, self-contained case: what `repro.sh` replays.
#[derive(Clone, Debug)]
struct Case {
    setup: String,
    specs: Vec<String>,
}

impl Case {
    fn to_json(&self, signature: &str) -> Value {
        json!({ "setup": self.setup, "specs": self.specs, "signature": signature })
    }
    fn from_json(v: &Value) -> Option<(Case, String)> {
        let setup = v["setup"].as_str()?.to_string();
        let specs = v["specs"].as_array()?.iter().map(|s| s.as_str().map(String::from)).collect::<Option<Vec<_>>>()?;
        Some((Case { setup, specs }, v["signature"].as_str().unwrap_or("").to_string()))
    }
}

#[derive(Clone, Debug)]
struct Fail {
    sig: String,
    msg: String,
    /// The single spec that failed, with the full case it came from.
    spec: String,
    case: Case,
}

enum Eval {
    Pass,
    Discard,
    Fail(Fail),
}

// ---------------------------------------------------------------- counters

macro_rules! counters {
    ($($name:ident),*) => {
        $(static $name: AtomicU64 = AtomicU64::new(0);)*
        fn counter_values() -> Vec<(&'static str, u64)> {
            vec![$((stringify!($name), $name.load(Ordering::Relaxed))),*]
        }
    };
}
counters!(
    KNOWN_DANGLING_SYMREF_PEEL_REGEX, KNOWN_BRANCH_SYMREF, KNOWN_TAG_BRANCH_AMBIGUOUS, KNOWN_LOCAL_UPSTREAM, GIT_FULL_REFNAME_SIBLING, GIT_PUSH_OF_MISSING_BRANCH, GIT_SUFFIX_AFTER_AT_INCONSISTENT, GIT_PEEL_ERROR_BUT_OK, GIT_REFLOG_UNEXPECTEDLY_ENDED, KNOWN_DETACHED_BARE_REFLOG, KNOWN_AT_AT, KNOWN_PREV_THEN_AT, KNOWN_SHA256_SHORT_HASH, KNOWN_MSGSEARCH_DETACHED_HEAD, CASES, FAILURES, SPECS, AGREE_OID, AGREE_ERR, SKIP_AMBIGUOUS_PREFIX, SETUP_FAILED, FASTPATH_OVERRULED, SHRINK_EVALS, QUERIES, AGREE_QUERY, DESCRIBE_NAME_TIE, KNOWN_TOPO_TIP_IS_END, KNOWN_DESCRIBE_TAG_CHAIN, GIT_DESCRIBE_EXTERNAL_NAME, GIT_REVLIST_SKEW_LIMITING, GRAPH_CAPPED, GIT_YOUNGEST_TIE, GIT_FP_ORDER_NEEDS_COMMIT_GRAPH, GIT_DESCRIBE_NOT_FEWEST, DIFF_AMBIGUOUS_RENAMES, DIFF_QUERIES, DIFF_WITH_RENAMES, DIFF_WITH_LINKS, KNOWN_TRAILING_SLASH_NON_TREE, MERGE_QUERIES, MERGE_CLEAN, MERGE_DIR_RENAME, GIT_RENAME_ADD_OF_SOURCE, MERGE_AMBIGUOUS_RENAMES, KNOWN_ADD_ADD_EXEC, WT_QUERIES, WT_DIRTY, WT_HEAD_CYCLE, GIT_ELOOP_DELETION, GIT_UNTRACKED_UNDER_ASSUME_UNCHANGED
);
fn bump(c: &AtomicU64) {
    c.fetch_add(1, Ordering::Relaxed);
}

// ---------------------------------------------------------------- environment

fn exe_dir() -> PathBuf {
    std::env::current_exe().expect("current exe").parent().expect("exe dir").to_path_buf()
}

/// Directory holding a `git` symlink to the reference git (created by build.sh).
fn git_bin_dir() -> PathBuf {
    exe_dir().join("gitbin")
}

fn git_env(cmd: &mut Command, home: &Path) {
    let path = format!("{}:{}", git_bin_dir().display(), std::env::var("PATH").unwrap_or_default());
    for (k, _) in std::env::vars() {
        if k.starts_with("GIT_") {
            cmd.env_remove(k);
        }
    }
    // The isolated environment of gix-testtools (tests/tools/src/lib.rs).
    cmd.env("PATH", path)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "author")
        .env("GIT_AUTHOR_EMAIL", "author@example.com")
        .env("GIT_COMMITTER_NAME", "committer")
        .env("GIT_COMMITTER_EMAIL", "committer@example.com")
        .env("GIT_AUTHOR_DATE", format!("{} +0000", gen::T0))
        .env("GIT_COMMITTER_DATE", format!("{} +0000", gen::T0))
        .env("GIT_CONFIG_COUNT", "3")
        .env("GIT_CONFIG_KEY_0", "commit.gpgsign")
        .env("GIT_CONFIG_VALUE_0", "false")
        .env("GIT_CONFIG_KEY_1", "maintenance.auto")
        .env("GIT_CONFIG_VALUE_1", "false")
        .env("GIT_CONFIG_KEY_2", "gc.auto")
        .env("GIT_CONFIG_VALUE_2", "0")
        .env("GIT_EDITOR", "true")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C");
}

struct TempDir(PathBuf);
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tempdir() -> TempDir {
    static N: AtomicU64 = AtomicU64::new(0);
    let base = std::env::var_os("TMPDIR").map(PathBuf::from).unwrap_or_else(|| "/tmp".into());
    let p = base.join(format!("gixfuzz-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("create temp dir");
    TempDir(p)
}

/// Run the setup script; returns the repo's worktree path.
fn setup(dir: &Path, script: &str) -> Option<PathBuf> {
    std::fs::write(dir.join("setup.sh"), script).ok()?;
    let mut cmd = Command::new("bash");
    cmd.arg("setup.sh").current_dir(dir).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    git_env(&mut cmd, dir);
    let st = cmd.status().ok()?;
    if st.code() == Some(99) || !dir.join("repo/.git").is_dir() {
        return None;
    }
    Some(dir.join("repo"))
}

fn git(repo: &Path, home: &Path, args: &[&str], stdin: Option<&str>) -> Option<(bool, String, String)> {
    let mut cmd = Command::new("git");
    // Leading `@NAME=value` words are environment variables for this call.
    let n = args.iter().take_while(|a| a.starts_with('@') && a.contains('=')).count();
    cmd.args(&args[n..]).current_dir(repo).stdout(Stdio::piped()).stderr(Stdio::piped());
    git_env(&mut cmd, home);
    for a in &args[..n] {
        let (k, v) = a[1..].split_once('=').unwrap_or_default();
        cmd.env(k, v);
    }
    cmd.stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() });
    let mut child = cmd.spawn().ok()?;
    if let Some(input) = stdin {
        let mut si = child.stdin.take()?;
        let input = input.to_string();
        // Write on a thread so a large output cannot deadlock against our input.
        std::thread::spawn(move || {
            let _ = si.write_all(input.as_bytes());
        });
    }
    let out = child.wait_with_output().ok()?;
    Some((out.status.success(), String::from_utf8_lossy(&out.stdout).into(), String::from_utf8_lossy(&out.stderr).into()))
}

fn all_objects(repo: &Path, home: &Path) -> Vec<String> {
    let Some((_, out, _)) = git(repo, home, &["cat-file", "--batch-all-objects", "--batch-check=%(objectname)"], None) else {
        return vec![];
    };
    let mut v: Vec<String> = out.lines().map(String::from).collect();
    v.sort();
    v
}

fn all_commits(repo: &Path, home: &Path) -> Vec<String> {
    let Some((_, out, _)) = git(repo, home, &["cat-file", "--batch-all-objects", "--batch-check=%(objectname) %(objecttype)"], None) else {
        return vec![];
    };
    let mut v: Vec<String> = out.lines().filter_map(|l| l.strip_suffix(" commit")).map(String::from).collect();
    v.sort();
    v
}

fn is_oid(s: &str) -> bool {
    (s.len() == 40 || s.len() == 64) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// git's answer for one spec: `git rev-parse --verify <spec>` (the oracle of record).
#[derive(Clone, Debug, PartialEq)]
enum GitAns {
    Oid(String),
    Err(String),
    /// git's answer is a fallback it warns about, not the documented value; no reference answer.
    Unreliable,
}

/// Whether git's "log for ref X unexpectedly ended on <date>" warning names the date of X's newest
/// reflog entry (read_ref_at visited no other entry before giving up on the log).
fn warns_about_newest_entry(repo: &Path, home: &Path, err: &str) -> bool {
    let Some(rest) = err.lines().find_map(|l| l.strip_prefix("warning: log for ref ")) else {
        return false;
    };
    let Some((refname, date)) = rest.split_once(" unexpectedly ended on ") else {
        return false;
    };
    let args = ["log", "-g", "-n1", "--date=rfc2822", "--format=%gd", "--end-of-options", refname];
    let Some((true, out, _)) = git(repo, home, &args, None) else {
        return false;
    };
    out.trim().rsplit_once("@{").is_some_and(|(_, newest)| newest.strip_suffix('}') == Some(date.trim()))
}

fn git_rev_parse(repo: &Path, home: &Path, spec: &str) -> Option<GitAns> {
    let (ok, out, err) = git(repo, home, &["rev-parse", "--verify", "--end-of-options", spec], None)?;
    let line = out.trim();
    // git bug: `X@{n}^{tag}` on a non-tag prints "error: ...: expected tag type, but the object
    // dereferences to tree type" and still exits 0 with X@{n}. git-rev-parse(1) says the peel must
    // "barf" when the type is not found, so this counts as an error.
    if ok && err.contains("type, but the object dereferences to") {
        bump(&GIT_PEEL_ERROR_BUT_OK);
        return Some(GitAns::Err(err.trim().to_string()));
    }
    // git artifact: `<ref>@{<date>}` is "the value of the ref at a prior point in time"
    // (gitrevisions(7)). When the reflog entry after that point has a null old id (an orphan
    // `commit (initial)` or a re-created branch), git's read_ref_at stops trusting the log, warns
    // "log for ref X unexpectedly ended on <date>" and returns the ref's *current* value instead of
    // the entry's new id. gix returns the entry's new id. git's answer is not a reference then.
    // Not so when the warning is about the *newest* entry: that only means the log is out of date
    // (e.g. a symbolic ref whose target moved), and the current value is what `<ref>@{0}` ("the
    // n-th prior value", gitrevisions(7)) and a date after the last entry mean.
    if ok && err.contains("unexpectedly ended on") && !warns_about_newest_entry(repo, home, &err) {
        bump(&GIT_REFLOG_UNEXPECTEDLY_ENDED);
        return Some(GitAns::Unreliable);
    }
    Some(if ok && is_oid(line) { GitAns::Oid(line.to_string()) } else { GitAns::Err(err.trim().to_string()) })
}

/// Fast path: `git cat-file --batch-check` for all specs. It resolves names with the same
/// get_oid machinery as rev-parse; any disagreement with gix is re-checked with rev-parse.
/// Some specs make git die (e.g. `@{9}` past the reflog); the batch is then restarted after
/// that spec, which stays undecided (`None`) for rev-parse to answer.
fn git_batch(repo: &Path, home: &Path, specs: &[String]) -> Vec<Option<Option<String>>> {
    let mut res = vec![None; specs.len()];
    if specs.iter().any(|s| s.contains('\n') || s.is_empty()) {
        return res;
    }
    let mut start = 0;
    while start < specs.len() {
        let mut input = specs[start..].join("\n");
        input.push('\n');
        let Some((_, out, _)) = git(repo, home, &["cat-file", "--batch-check=%(objectname)"], Some(&input)) else {
            return res;
        };
        let lines: Vec<&str> = out.lines().collect();
        for (j, l) in lines.iter().enumerate().take(specs.len() - start) {
            res[start + j] = Some(if is_oid(l) { Some(l.to_string()) } else { None });
        }
        start += lines.len() + 1;
    }
    res
}

// ---------------------------------------------------------------- watchdog & panic capture

const GIX_TIMEOUT: Duration = Duration::from_secs(20);

struct Watch {
    since: Instant,
    case: Case,
}
static WATCH: Mutex<Option<Watch>> = Mutex::new(None);
static OUT_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

thread_local! {
    static PANIC_AT: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let at = info.location().map(|l| l.file().to_string());
        PANIC_AT.with(|p| *p.borrow_mut() = at);
        previous(info);
    }));
}

/// `crash__panic_<crate>`, the crate taken from the panic location, so distinct crates do
/// not share one signature (and one 3-case cap).
fn panic_sig(file: Option<String>) -> String {
    let file = file.unwrap_or_default();
    let krate = file
        .split('/')
        .collect::<Vec<_>>()
        .windows(2)
        .rev()
        .find(|w| w[1] == "src" && w[0].starts_with("gix"))
        .map(|w| w[0].replace('-', "_"))
        .unwrap_or_else(|| "other".into());
    format!("crash__panic_{krate}")
}

fn panic_message(p: &(dyn std::any::Any + Send)) -> String {
    p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_default()
}

// ---------------------------------------------------------------- the oracle

#[derive(Debug)]
enum GixAns {
    Oid(String),
    Err(String),
    Panic(String, Option<String>),
    /// Answer to a `graph` query (`#...` spec).
    Query(graph::Ans),
}

fn gix_answers(repo_dir: &Path, specs: &[String], case: &Case) -> Vec<GixAns> {
    *WATCH.lock().unwrap() = Some(Watch { since: Instant::now(), case: case.clone() });
    let answers = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        gix::open_opts(repo_dir, gix::open::Options::isolated()).map_err(|e| e.to_string())
    }));
    let out = match answers {
        Err(p) => {
            let at = PANIC_AT.with(|a| a.borrow_mut().take());
            specs.iter().map(|_| GixAns::Panic(panic_message(&*p), at.clone())).collect()
        },
        Ok(Err(e)) => specs.iter().map(|_| GixAns::Err(format!("open: {e}"))).collect(),
        Ok(Ok(repo)) => specs
            .iter()
            .map(|spec| {
                if let Some(q) = graph::parse(spec) {
                    return match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| graph::gix_answer(&repo, &q))) {
                        Ok(a) => GixAns::Query(a),
                        Err(p) => GixAns::Panic(panic_message(&*p), PANIC_AT.with(|a| a.borrow_mut().take())),
                    };
                }
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    repo.rev_parse_single(spec.as_str()).map(|id| id.to_string()).map_err(|e| e.to_string())
                })) {
                    Ok(Ok(id)) => GixAns::Oid(id),
                    Ok(Err(e)) => GixAns::Err(e),
                    Err(p) => GixAns::Panic(panic_message(&*p), PANIC_AT.with(|a| a.borrow_mut().take())),
                }
            })
            .collect(),
    };
    *WATCH.lock().unwrap() = None;
    out
}

/// Evaluate a concrete case. The first disagreeing spec is reported.
fn eval_case(case: &Case) -> Eval {
    let dir = tempdir();
    let Some(repo) = setup(&dir.0, &case.setup) else {
        bump(&SETUP_FAILED);
        return Eval::Discard;
    };
    eval_specs(&dir.0, &repo, case)
}

/// Refs as files and packed-refs say (no git call; dangling symrefs included).
#[derive(Default)]
struct RefInfo {
    branches: std::collections::BTreeSet<String>,
    tags: std::collections::BTreeSet<String>,
    symref_branches: std::collections::BTreeSet<String>,
}

fn ref_info(repo: &Path) -> RefInfo {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<(String, String)>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let name = format!("{prefix}{}", e.file_name().to_string_lossy());
            if e.path().is_dir() {
                walk(&e.path(), &format!("{name}/"), out);
            } else {
                out.push((name, std::fs::read_to_string(e.path()).unwrap_or_default()));
            }
        }
    }
    let git = repo.join(".git");
    let mut info = RefInfo::default();
    let mut files = Vec::new();
    walk(&git.join("refs/heads"), "", &mut files);
    for (n, content) in files {
        if content.starts_with("ref:") {
            info.symref_branches.insert(n.clone());
        }
        info.branches.insert(n);
    }
    let mut files = Vec::new();
    walk(&git.join("refs/tags"), "", &mut files);
    info.tags.extend(files.into_iter().map(|(n, _)| n));
    for line in std::fs::read_to_string(git.join("packed-refs")).unwrap_or_default().lines() {
        if let Some((_, name)) = line.split_once(' ') {
            if let Some(b) = name.strip_prefix("refs/heads/") {
                info.branches.insert(b.into());
            } else if let Some(t) = name.strip_prefix("refs/tags/") {
                info.tags.insert(t.into());
            }
        }
    }
    info
}

/// The ref-name part of a spec before any `@{`, `~`, `^`, `:`.
fn spec_base(spec: &str) -> &str {
    let end = spec.find(|c| c == '~' || c == '^' || c == ':').unwrap_or(spec.len());
    let end = spec[..end].find("@{").unwrap_or(end);
    &spec[..end]
}

/// Known gix bugs found by this oracle, masked so they do not drown everything else. Each is
/// narrow, counted in stats.json, and off with GIXFUZZ_NO_MASK=1 (use that when re-checking a fix).
fn known_bug(sig: &str, spec: &str, gix_err: Option<&str>, git_err: Option<&str>, repo: &Path) -> Option<&'static AtomicU64> {
    if std::env::var_os("GIXFUZZ_NO_MASK").is_some() {
        return None;
    }
    // B14: `<rev>:<path>/` where <path> is a blob or symlink: git requires a tree there
    // ("Needed a single revision"), gix returns the entry. Masked once this process wrote its
    // KNOWN_CAP findings for the signature (like the graph masks).
    if sig == "revspec__gix_accepts_tree_path" && spec.ends_with('/') && OUT_DIR.lock().unwrap().as_ref().is_some_and(|out| sig_count(out, sig) >= KNOWN_CAP) {
        return Some(&KNOWN_TRAILING_SLASH_NON_TREE);
    }
    let config = std::fs::read_to_string(repo.join(".git/config")).unwrap_or_default();
    let head = std::fs::read_to_string(repo.join(".git/HEAD")).unwrap_or_default();
    // B1: in SHA-256 repos gix rejects every abbreviated object id ("couldn't parse revision").
    if config.contains("objectformat = sha256")
        && sig.starts_with("revspec__gix_rejects")
        && gix_err.is_some_and(|e| e.contains("couldn't parse revision"))
    {
        return Some(&KNOWN_SHA256_SHORT_HASH);
    }
    let rejects = sig.starts_with("revspec__gix_rejects");
    let kind = sig.rsplit('_').next().unwrap_or("");
    let kind = if sig.ends_with("prev_checkout") { "prev_checkout" } else if sig.ends_with("peel_regex") { "peel_regex" } else { kind };
    let refs = ref_info(repo);
    let at_kinds = ["reflog", "date", "prev_checkout", "upstream", "push"];
    // B13: symbolic refs among branches (refs/heads/x -> refs/heads/y, dangling or looping):
    // reflog / previous-branch / sibling lookups and `^{/re}` disagree with git in many ways.
    if !refs.symref_branches.is_empty() && (at_kinds.contains(&kind) || spec.contains("@{") || spec.contains("^{/")) {
        return Some(&KNOWN_BRANCH_SYMREF);
    }
    // B9/B10: a name that is both a tag and a branch: git's reflog lookup (dwim_log) and
    // `@{-n}` use the branch, gix picks the tag.
    if refs.branches.intersection(&refs.tags).next().is_some() && spec.contains("@{") {
        return Some(&KNOWN_TAG_BRANCH_AMBIGUOUS);
    }
    // B12: upstream / push of a branch whose upstream is local (`branch.<b>.remote = .`):
    // gix fails in sibling_branch().
    if rejects && config.contains("remote = .") && spec.to_ascii_lowercase().contains("@{u") || rejects && config.contains("remote = .") && spec.contains("@{push}") {
        return Some(&KNOWN_LOCAL_UPSTREAM);
    }
    let _ = spec_base;
    // B11: `ref^{/re}` where ref is a dangling symref: git fails ("ignoring dangling symref"),
    // gix searches from all refs as if it were `:/re`.
    if sig.starts_with("revspec__gix_accepts") && spec.contains("^{/") && git_err.is_some_and(|e| e.contains("ignoring dangling symref")) {
        return Some(&KNOWN_DANGLING_SYMREF_PEEL_REGEX);
    }
    let detached = !head.starts_with("ref:");
    // B5: bare `@{n}` / `@{date}` with a detached HEAD: git uses HEAD's reflog, gix errors
    // ("Unborn heads do not have a reflog yet", revision.rs reflog()).
    if rejects && detached && spec.starts_with("@{") && !spec.starts_with("@{-") {
        return Some(&KNOWN_DETACHED_BARE_REFLOG);
    }
    // B7: `@@{n}` (git: `@` is HEAD, so HEAD@{n}) does not parse in gix.
    if rejects && spec.starts_with("@@{") && gix_err.is_some_and(|e| e.contains("couldn't parse revision")) {
        return Some(&KNOWN_AT_AT);
    }
    // B6: `@{-n}@{...}` (reflog/upstream of the n-th previous branch): gix "unconsumed input".
    if rejects && spec.starts_with("@{-") && spec.contains("}@{") && gix_err.is_some_and(|e| e.contains("unconsumed input")) {
        return Some(&KNOWN_PREV_THEN_AT);
    }
    // B2: `:/re` does not walk from a detached HEAD (gix/src/revision/spec/parse/delegate/navigate.rs
    // uses references().all()); git: "reachable from any ref, including HEAD".
    if spec.starts_with(":/") && !head.starts_with("ref:") && !sig.starts_with("revspec__gix_accepts") {
        return Some(&KNOWN_MSGSEARCH_DETACHED_HEAD);
    }
    None
}

/// Findings a search process writes for a known-bug signature (graph/treediff/merge queries, B14)
/// before further disagreements are counted instead of failed. One per process: the controller
/// runs many processes, and the time a failure costs (shrinking) is better spent searching.
const KNOWN_CAP: usize = 1;
const WT_FAMILY_CAP: usize = 2;

/// Known gix bugs found by the graph oracle. Unlike the revspec masks these only apply once this
/// search process has written its KNOWN_CAP findings for the signature (so every run still reports them,
/// and repro.sh, which has no output dir, never masks); after that they are counted, not failed.
fn known_graph_bug(sig: &str, q: &graph::Query<'_>, gix: &graph::Ans, git_ans: &graph::Ans, run: &dyn Fn(&[&str]) -> Option<(bool, String, String)>) -> Option<&'static AtomicU64> {
    if std::env::var_os("GIXFUZZ_NO_MASK").is_some() {
        return None;
    }
    // Per oracle family (graph__, merge__, treediff__) too: real bugs there are many and frequent
    // (G1-G6, merge conflicts), and every process reporting each of them once is above 1% of the
    // cases of a short run; other processes and windows report the others.
    let family = sig.split("__").next().unwrap_or(sig);
    // The status/index oracles' areas are distinct enough to cap per signature only.
    // (at most WT_FAMILY_CAP of them per process).
    let family_cap = if matches!(family, "status" | "index") { WT_FAMILY_CAP } else { KNOWN_CAP };
    let capped = OUT_DIR.lock().unwrap().as_ref().is_some_and(|out| sig_count(out, sig) >= KNOWN_CAP || family_count(out, family) >= family_cap);
    if !capped {
        return None;
    }
    // G1: gix_traverse topo walk returns a commit that is both a tip and an end; `git rev-list
    // X ^X` lists nothing (topo/init.rs documents "Like `git rev-list --topo-order ^ends tips`").
    if q.kind == "topo" && q.words.iter().any(|w| w.starts_with('^') && q.words.contains(&&w[1..])) {
        return Some(&KNOWN_TOPO_TIP_IS_END);
    }
    // G2: describe: git's name is an annotated tag pointing to another tag; gix only takes tags
    // whose target is the commit itself (gix/src/commit/mod.rs SelectRef::names()).
    if q.kind == "describe" {
        if let (Ok(_), Ok(b)) = (gix, git_ans) {
            let name = b.first().and_then(|l| l.rsplit_once(' ')).map(|(n, _)| n.to_string()).unwrap_or_default();
            if let Some((true, body, _)) = run(&["cat-file", "-p", &format!("refs/tags/{name}")]) {
                if let Some(target) = body.lines().next().and_then(|l| l.strip_prefix("object ")) {
                    if run(&["cat-file", "-t", target]).is_some_and(|(_, t, _)| t.trim() == "tag") {
                        return Some(&KNOWN_DESCRIBE_TAG_CHAIN);
                    }
                }
            }
        }
    }
    // Any other disagreement on a query signature this process already reported KNOWN_CAP
    // times: counted, not failed.
    Some(&GRAPH_CAPPED)
}

/// git's answer to a graph query in a copy of `repo` that has a full commit-graph.
fn git_answer_with_commit_graph(repo: &Path, home: &Path, q: &graph::Query<'_>) -> Option<graph::Ans> {
    static N: AtomicU64 = AtomicU64::new(0);
    let tmp = std::env::temp_dir().join(format!("gixfuzz-cg-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&tmp);
    let copied = Command::new("cp").arg("-a").arg(repo).arg(&tmp).status().ok()?.success();
    let ans = copied
        .then(|| {
            git(&tmp, home, &["commit-graph", "write", "--reachable"], None).filter(|r| r.0)?;
            let run = |args: &[&str]| {
                let mut a = vec!["-c", "core.commitGraph=true"];
                a.extend(args);
                git(&tmp, home, &a, None)
            };
            graph::git_answer(&run, q)
        })
        .flatten();
    let _ = std::fs::remove_dir_all(&tmp);
    ans
}

fn eval_specs(home: &Path, repo: &Path, case: &Case) -> Eval {
    let specs = &case.specs;
    if let Some(out) = OUT_DIR.lock().unwrap().as_ref() {
        let _ = std::fs::write(out.join("current.json"), case.to_json("crash__abort").to_string());
    }
    let gix = gix_answers(repo, specs, case);
    // Only gix runs under current.json: a death elsewhere (git, bash) is not gix's.
    if let Some(out) = OUT_DIR.lock().unwrap().as_ref() {
        let _ = std::fs::remove_file(out.join("current.json"));
    }
    let batch = git_batch(repo, home, specs);
    for (i, (spec, g)) in specs.iter().zip(&gix).enumerate() {
        bump(&SPECS);
        let gix_err = match g {
            GixAns::Err(e) => Some(e.as_str()),
            _ => None,
        };
        let fail = |sig: &str, msg: String, git_err: Option<&str>| {
            if let Some(c) = known_bug(sig, spec, gix_err, git_err, repo).filter(|_| sig.starts_with("revspec")) {
                bump(c);
                return None;
            }
            Some(Eval::Fail(Fail { sig: sig.into(), msg, spec: spec.clone(), case: case.clone() }))
        };
        if let GixAns::Panic(m, at) = g {
            if let Some(f) = fail(&panic_sig(at.clone()), format!("gix panicked on {spec:?}: {m} at {}", at.clone().unwrap_or_default()), None) {
                    return f;
                }
                continue;
        }
        if let GixAns::Query(ga) = g {
            bump(&QUERIES);
            let Some(q) = graph::parse(spec) else { continue };
            let run = |args: &[&str]| git(repo, home, args, None);
            let Some(want) = graph::git_answer(&run, &q) else { return Eval::Discard };
            if want.as_ref().is_ok_and(|w| w.first().is_some_and(|l| l == graph::EXTERNAL_NAME)) {
                bump(&GIT_DESCRIBE_EXTERNAL_NAME);
                continue;
            }
            if matches!(q.kind, "status" | "index" | "index_rt") {
                bump(&WT_QUERIES);
                if ga.as_ref().is_err_and(|e| e.starts_with(wt::HEAD_CYCLE)) {
                    bump(&WT_HEAD_CYCLE);
                    continue;
                }
                if q.kind == "status" && wt::git_misses_deletion_under_symlink_loop(repo, ga, &want) {
                    bump(&GIT_ELOOP_DELETION);
                    continue;
                }
                if q.kind == "status" && ga != &want && wt::git_untracked_under_assume_unchanged(&run, ga, &want) {
                    bump(&GIT_UNTRACKED_UNDER_ASSUME_UNCHANGED);
                    continue;
                }
                if q.kind == "status" && want.as_ref().is_ok_and(|w| !w.is_empty()) {
                    bump(&WT_DIRTY);
                }
            }
            if q.kind == "merge" {
                bump(&MERGE_QUERIES);
                if want.as_ref().is_ok_and(|w| w.first().is_some_and(|l| l.starts_with("clean"))) {
                    bump(&MERGE_CLEAN);
                }
                if want.as_ref().is_ok_and(|w| w.first().is_some_and(|l| l == merge::DIR_RENAME)) {
                    bump(&MERGE_DIR_RENAME);
                    continue;
                }
                if ga != &want && merge::git_rename_add_of_source(&run, &q) {
                    bump(&GIT_RENAME_ADD_OF_SOURCE);
                    continue;
                }
                if ga != &want && merge::ambiguous_exact_renames(&run, &q) {
                    bump(&MERGE_AMBIGUOUS_RENAMES);
                    continue;
                }
            }
            let masked = if q.kind == "merge" { merge::mask_add_add_exec(&run, &q, ga, &want) } else { None };
            if masked.is_some() {
                bump(&KNOWN_ADD_ADD_EXEC);
            }
            let ga = masked.as_ref().unwrap_or(ga);
            if q.kind == "diff" {
                bump(&DIFF_QUERIES);
                if let Ok(w) = &want {
                    if w.iter().any(|l| l.starts_with("R ")) {
                        bump(&DIFF_WITH_RENAMES);
                    }
                    if w.iter().any(|l| l.contains("120000") || l.contains("160000")) {
                        bump(&DIFF_WITH_LINKS);
                    }
                }
            }
            if want.as_ref().is_ok_and(|w| w.first().is_some_and(|l| l == treediff::AMBIGUOUS)) {
                bump(&DIFF_AMBIGUOUS_RENAMES);
                continue;
            }
            match graph::compare(&q, ga, &want) {
                None => bump(&AGREE_QUERY),
                Some((kind, msg)) => {
                    // git's rev-list answer is not the exact set (date-limiting under clock skew):
                    // judge gix by the exact set; an order difference then has no reference.
                    if kind == "wrong" && (q.kind == "walk" || q.kind == "topo") {
                        if let Some(exact) = graph::exact_set(&run, &q) {
                            if graph::sorted(&want).as_ref() != Some(&exact) {
                                bump(&GIT_REVLIST_SKEW_LIMITING);
                                if graph::sorted(ga).as_ref() == Some(&exact) {
                                    continue;
                                }
                            }
                        }
                    }
                    // git's `--first-parent --topo-order|--date-order` order depends on whether it
                    // reads a commit-graph (black box, git 2.5x: without one, a merge's other parents
                    // still wait for the merge; with one they don't, e.g. `rev-list --first-parent
                    // --topo-order main side` where side is merged into main). gix walks like git with a
                    // commit-graph either way and is baselined against that (gix-traverse topo tests).
                    // Same set and git with a commit-graph gives gix's order: no reference for the order.
                    if kind == "wrong"
                        && q.kind == "topo"
                        && q.words.get(1) == Some(&"fp")
                        && graph::sorted(ga).is_some()
                        && graph::sorted(ga) == graph::sorted(&want)
                        && git_answer_with_commit_graph(repo, home, &q).is_some_and(|a| a.as_ref().ok() == ga.as_ref().ok())
                    {
                        bump(&GIT_FP_ORDER_NEEDS_COMMIT_GRAPH);
                        continue;
                    }
                    if kind == "wrong" && q.kind == "describe" && graph::describe_tie(&run, ga, &want) {
                        bump(&DESCRIBE_NAME_TIE);
                        continue;
                    }
                    if kind == "wrong" && q.kind == "describe" && graph::describe_git_not_fewest(&run, &q, ga, &want) {
                        bump(&GIT_DESCRIBE_NOT_FEWEST);
                        continue;
                    }
                    let sig = graph::signature(kind, &q, ga, &want);
                    if let Some(c) = known_graph_bug(&sig, &q, ga, &want, &run) {
                        bump(c);
                        continue;
                    }
                    if let Some(f) = fail(&sig, msg, None) {
                        return f;
                    }
                },
            }
            continue;
        }
        // Fast path agrees: done.
        if let Some(b) = &batch[i] {
            match (g, b) {
                (GixAns::Oid(x), Some(y)) if x == y => {
                    bump(&AGREE_OID);
                    continue;
                },
                (GixAns::Err(_), None) => {
                    bump(&AGREE_ERR);
                    continue;
                },
                _ => {},
            }
        }
        let Some(mut want) = git_rev_parse(repo, home, spec) else { return Eval::Discard };
        if matches!(want, GitAns::Unreliable) {
            continue;
        }
        // git bug: after `ref@{...}`, a failing `^{/re}` or `^{type}` makes git return the ref's
        // current value (exit 0). Suffixes apply to the object the prefix names
        // (git-rev-parse(1)), so the reference answer is git's answer for `<oid of prefix><suffix>`.
        if let Some((prefix, rest)) = split_after_at_forms(spec) {
            if let Some(GitAns::Oid(p)) = git_rev_parse(repo, home, prefix) {
                if let Some(alt) = git_rev_parse(repo, home, &format!("{p}{rest}")) {
                    let same = match (&alt, &want) {
                        (GitAns::Oid(a), GitAns::Oid(b)) => a == b,
                        (GitAns::Err(_), GitAns::Err(_)) => true,
                        _ => false,
                    };
                    if !same {
                        bump(&GIT_SUFFIX_AFTER_AT_INCONSISTENT);
                        want = alt;
                    }
                }
            }
        }
        match (g, &want) {
            (GixAns::Oid(x), GitAns::Oid(y)) if x == y => {
                if std::env::var_os("GIXFUZZ_DEBUG").is_some() {
                    eprintln!("overruled (oid): {spec:?} batch={:?}", batch[i]);
                }
                bump(&FASTPATH_OVERRULED);
                bump(&AGREE_OID);
            },
            (GixAns::Err(_), GitAns::Err(_)) => {
                if std::env::var_os("GIXFUZZ_DEBUG").is_some() {
                    eprintln!("overruled (err): {spec:?} batch={:?}", batch[i]);
                }
                bump(&FASTPATH_OVERRULED);
                bump(&AGREE_ERR);
            },
            (GixAns::Oid(x), GitAns::Oid(y)) => {
                // `:/re` and `^{/re}` name "the youngest matching commit" (gitrevisions(7)). With
                // repeated or skewed commit times (ClockBack), git returns the first match of its
                // date-ordered walk; a gix answer at least as young is no contradiction of the docs.
                if matches!(spec_kind(spec), "msgsearch" | "peel_regex") {
                    let ct = |id: &str| git(repo, home, &["log", "-1", "--format=%ct", id], None).and_then(|(_, o, _)| o.trim().parse::<i64>().ok());
                    if let (Some(a), Some(b)) = (ct(x), ct(y)) {
                        if a >= b {
                            bump(&GIT_YOUNGEST_TIE);
                            continue;
                        }
                    }
                }
                if let Some(f) = fail(&format!("revspec__wrong_object_{}", spec_kind(spec)), format!("spec {spec:?}: gix -> {x}, git rev-parse --verify -> {y}"), None) {
                    return f;
                }
                continue;
            },
            (GixAns::Err(e), GitAns::Oid(y)) => {
                // `<branchname>@{push}` names "where we would push to if git push were run while
                // branchname was checked out" (gitrevisions(7)); git itself refuses a branch that does
                // not exist ("fatal: no such branch: 'x'" for `x@{u}`, and for `x@{push}` under
                // push.default=simple/upstream). Only push.default=current/matching skip that check in
                // git (remote.c branch_get_push_1), so git's oid there is an artifact, not a contract.
                if push_of_missing_branch(spec, repo) {
                    bump(&GIT_PUSH_OF_MISSING_BRANCH);
                    continue;
                }
                if let Some(f) = fail(&format!("revspec__gix_rejects_{}", spec_kind(spec)), format!("spec {spec:?}: gix error {e:?}, git rev-parse --verify -> {y}"), None) {
                    return f;
                }
                continue;
            },
            (GixAns::Oid(x), GitAns::Err(e)) => {
                // git only accepts a short branch name before @{upstream}/@{push} (`<branchname>@{upstream}`
                // in gitrevisions(7)); `refs/heads/x@{push}`, `heads/x@{u}` make it say "no such branch",
                // or, for @{push} under push.default=current/matching (no branch-existence check in
                // remote.c branch_get_push_1), just "Needed a single revision".
                if e.contains("no such branch: 'refs/heads/") || e.contains("no such branch: 'heads/") || full_refname_sibling(spec, repo) {
                    bump(&GIT_FULL_REFNAME_SIBLING);
                    continue;
                }
                // Documented superset: gix disambiguates some short ids git calls ambiguous
                // (gix/tests/gix/revision/spec/from_bytes/util.rs:139 "Git can't do that, but we can").
                if e.contains("short object ID") && e.contains("is ambiguous") {
                    bump(&SKIP_AMBIGUOUS_PREFIX);
                    continue;
                }
                // `<rev>:<path>/` naming a non-tree (B14) is about the path, whatever comes before it.
                let kind = if spec.ends_with('/') && slash_names_non_tree(repo, home, spec) { "tree_path" } else { spec_kind(spec) };
                if let Some(f) = fail(&format!("revspec__gix_accepts_{kind}"), format!("spec {spec:?}: gix -> {x}, git rev-parse --verify fails: {e:?}"), Some(e.as_str())) {
                    return f;
                }
                continue;
            },
            (GixAns::Panic(..) | GixAns::Query(_), _) | (_, GitAns::Unreliable) => unreachable!(),
        }
    }
    Eval::Pass
}

fn eval_scenario(s: &Scenario) -> Eval {
    let dir = tempdir();
    let setup_script = gen::render_setup(s);
    let Some(repo) = setup(&dir.0, &setup_script) else {
        bump(&SETUP_FAILED);
        return Eval::Discard;
    };
    let objects = all_objects(&repo, &dir.0);
    let mut specs: Vec<String> = s.specs.iter().map(|sp| gen::render_spec(sp, &objects)).collect();
    if !s.queries.is_empty() {
        let commits = all_commits(&repo, &dir.0);
        specs.extend(s.queries.iter().filter_map(|q| gen::render_query(q, &commits)));
    }
    eval_specs(&dir.0, &repo, &Case { setup: setup_script, specs })
}

/// git resolves `spec` without its trailing `/` to something that is not a tree.
fn slash_names_non_tree(repo: &Path, home: &Path, spec: &str) -> bool {
    let Some(GitAns::Oid(id)) = git_rev_parse(repo, home, spec.trim_end_matches('/')) else { return false };
    git(repo, home, &["cat-file", "-t", &id], None).is_some_and(|(ok, t, _)| ok && t.trim() != "tree")
}

/// `refs/heads/<b>@{push|u}...` or `heads/<b>@{...}` where the part before `@{` is not itself a branch
/// name (git looks it up as the branch `refs/heads/refs/heads/<b>`).
fn full_refname_sibling(spec: &str, repo: &Path) -> bool {
    let Some(at) = spec.find("@{") else { return false };
    let (b, rest) = (&spec[..at], spec[at..].to_ascii_lowercase());
    let mark = rest.starts_with("@{push}") || rest.starts_with("@{u}") || rest.starts_with("@{upstream}");
    mark && (b.starts_with("refs/heads/") || b.starts_with("heads/")) && !ref_info(repo).branches.contains(b)
}

/// `<b>@{push}...` where `<b>` is a short branch name and refs/heads/<b> does not exist.
fn push_of_missing_branch(spec: &str, repo: &Path) -> bool {
    let Some(at) = spec.to_ascii_lowercase().find("@{push}") else { return false };
    let b = &spec[..at];
    !b.is_empty() && b != "HEAD" && b != "@" && !b.contains("@{") && !ref_info(repo).branches.contains(b)
}

/// `ref@{..}@{..}rest` -> (`ref@{..}@{..}`, `rest`) when `rest` is not empty.
fn split_after_at_forms(spec: &str) -> Option<(&str, &str)> {
    let mut end = spec.find("@{")?;
    while spec[end..].starts_with("@{") {
        end += spec[end..].find('}')? + 1;
    }
    let (p, r) = spec.split_at(end);
    (!r.is_empty()).then_some((p, r))
}

/// The construct a failing spec exercises, most specific first, so one bug class keeps one
/// signature (shrinking keeps the signature, hence the construct).
fn spec_kind(spec: &str) -> &'static str {
    let l = spec.to_ascii_lowercase();
    if spec.starts_with(":/") {
        "msgsearch"
    } else if spec.contains("^{/") {
        "peel_regex"
    } else if spec.contains("@{-") {
        "prev_checkout"
    } else if l.contains("@{push}") {
        "push"
    } else if l.contains("@{u}") || l.contains("@{upstream}") {
        "upstream"
    } else if spec.contains("@{") {
        let reflog_n = spec.split("@{").skip(1).any(|r| r.split('}').next().is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())));
        if reflog_n && !spec.split("@{").skip(1).any(|r| r.split('}').next().is_some_and(|n| n.contains(':'))) { "reflog" } else { "date" }
    } else if spec.starts_with(':') {
        "index_path"
    } else if spec.contains(':') {
        "tree_path"
    } else if spec.contains("^{") {
        "peel"
    } else if spec.contains('~') || spec.contains('^') {
        "nav"
    } else if spec.len() >= 4 && spec.bytes().all(|b| b.is_ascii_hexdigit()) {
        "hex"
    } else {
        "ref"
    }
}

/// Each signature panics at its own line, so ephyra's "same failure" (same panic site) matches ours.
fn raise(f: &Fail) -> ! {
    let s = f.sig.as_str();
    if s.starts_with("revspec__wrong_object") {
        panic!("[{s}] {}", f.msg)
    } else if s.starts_with("revspec__gix_rejects") {
        panic!("[{s}] {}", f.msg)
    } else if s.starts_with("revspec__gix_accepts") {
        panic!("[{s}] {}", f.msg)
    } else if s.starts_with("graph__wrong") {
        panic!("[{s}] {}", f.msg)
    } else if s.starts_with("graph__") {
        panic!("[{s}] {}", f.msg)
    } else {
        panic!("[{s}] {}", f.msg)
    }
}

// ---------------------------------------------------------------- findings & stats

fn write_stats(out: &Path, base: &BTreeMap<String, u64>) {
    let mut m = serde_json::Map::new();
    let vals = counter_values();
    for (k, v) in &vals {
        let key = k.to_lowercase();
        let b = base.get(&key).copied().unwrap_or(0);
        m.insert(key, json!(b + v));
    }
    let tmp = out.join("stats.json.tmp");
    let _ = std::fs::write(&tmp, Value::Object(m).to_string());
    let _ = std::fs::rename(&tmp, out.join("stats.json"));
}

fn read_stats(out: &Path) -> BTreeMap<String, u64> {
    std::fs::read_to_string(out.join("stats.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v.as_object().cloned())
        .map(|o| o.into_iter().filter_map(|(k, v)| v.as_u64().map(|v| (k, v))).collect())
        .unwrap_or_default()
}

/// Signatures with findings in `out` whose oracle family (the part before `__`) is `family`.
fn family_count(out: &Path, family: &str) -> usize {
    let prefix = format!("{family}__");
    std::fs::read_dir(out.join("findings")).map(|d| d.flatten().filter(|e| e.file_name().to_string_lossy().starts_with(&prefix)).count()).unwrap_or(0)
}

fn sig_count(out: &Path, sig: &str) -> usize {
    std::fs::read_dir(out.join("findings").join(sig)).map(|d| d.count()).unwrap_or(0)
}

const REPRO_SH: &str = "#!/bin/bash\n# Replays case.json: builds the repo with the reference git, compares gix with git.\n# Exit 1: reproduces; 0: does not; other: cannot decide.\ncd \"$(dirname \"$0\")\" || exit 2\nexec \"$1/bin/gixfuzz\" check case.json\n";

fn write_finding(out: &Path, sig: &str, msg: &str, seed: u64, case: &Case) {
    let n = sig_count(out, sig);
    if n >= 3 {
        return;
    }
    let dir = out.join("findings").join(sig).join(n.to_string());
    let _ = std::fs::create_dir_all(&dir);
    let oracle = sig.split("__").next().unwrap_or(sig);
    let _ = std::fs::write(
        dir.join("finding.json"),
        serde_json::to_string_pretty(&json!({"signature": sig, "oracle": oracle, "message": msg, "seed": seed})).unwrap(),
    );
    let _ = std::fs::write(dir.join("case.json"), serde_json::to_string_pretty(&case.to_json(sig)).unwrap());
    let _ = std::fs::write(dir.join("setup.sh"), &case.setup);
    let _ = std::fs::write(dir.join("specs.txt"), case.specs.join("\n") + "\n");
    let repro = dir.join("repro.sh");
    let _ = std::fs::write(&repro, REPRO_SH);
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(&repro, std::fs::Permissions::from_mode(0o755));
}

fn spawn_watchdog(out: PathBuf, seed: u64, base: BTreeMap<String, u64>) {
    std::thread::spawn(move || {
        let mut last_stats = Instant::now();
        loop {
            std::thread::sleep(Duration::from_millis(250));
            let hung = WATCH.lock().unwrap().as_ref().filter(|w| w.since.elapsed() > GIX_TIMEOUT).map(|w| w.case.clone());
            if let Some(case) = hung {
                bump(&CASES);
                bump(&FAILURES);
                write_finding(&out, "crash__timeout", &format!("gix took more than {GIX_TIMEOUT:?} on one of the specs"), seed, &case);
                write_stats(&out, &base);
                let _ = std::fs::remove_file(out.join("current.json"));
                std::process::exit(3);
            }
            if last_stats.elapsed() > Duration::from_secs(5) {
                write_stats(&out, &base);
                last_stats = Instant::now();
            }
        }
    });
}

/// Greedy shrinking on our own signature (exact match, not just the panic site).
fn shrink(s: &Scenario, sig: &str, deadline: Instant) -> Scenario {
    let mut best = s.clone();
    let limit = Instant::now() + Duration::from_secs(20);
    let mut evals = 0;
    'outer: loop {
        let mut cands = Vec::new();
        let _ = gen::shrink_scenario(&best, &mut |c| {
            cands.push(c);
            std::ops::ControlFlow::Continue(())
        });
        for c in cands {
            if Instant::now() > limit || Instant::now() > deadline || evals > 400 {
                break 'outer;
            }
            evals += 1;
            bump(&SHRINK_EVALS);
            if let Eval::Fail(f) = eval_scenario(&c) {
                if f.sig == sig {
                    best = c;
                    continue 'outer;
                }
            }
        }
        break;
    }
    best
}

/// The finding's case: the failing spec alone when that still fails the same way.
fn minimal_case(f: &Fail) -> Case {
    let single = Case { setup: f.case.setup.clone(), specs: vec![f.spec.clone()] };
    match eval_case(&single) {
        Eval::Fail(g) if g.sig == f.sig => single,
        _ => f.case.clone(),
    }
}

fn splitmix(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x2545_F491_4F6C_DD1D);
    z = (z ^ (z >> 33)).wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    z = (z ^ (z >> 33)).wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    z ^ (z >> 33)
}

fn search(seed: u64, seconds: u64, out: PathBuf) {
    std::fs::create_dir_all(out.join("findings")).expect("out dir");
    let base = read_stats(&out);
    *OUT_DIR.lock().unwrap() = Some(out.clone());
    // Make ephyra install its panic hook first, so ours wraps it and sees every panic.
    let _ = ephyra::check(&Config::default().tests(1).no_regressions(), |_: &mut ephyra::Source| (), |_: ()| {});
    install_panic_hook();
    spawn_watchdog(out.clone(), seed, base.clone());
    let deadline = Instant::now() + Duration::from_secs(seconds);
    let mut k = 0u64;
    while Instant::now() < deadline {
        // Hash (seed, k): ephyra mixes seed + iteration * golden-ratio, so a linear seed
        // schedule would make different seeds replay shifted copies of one sequence.
        let run_seed = splitmix(splitmix(seed) ^ splitmix(k.wrapping_add(0x5151)));
        k += 1;
        let cfg = Config::default()
            .seed(run_seed)
            .tests(1 << 40)
            .budget(deadline.saturating_duration_since(Instant::now()))
            .no_regressions();
        let last: Mutex<Option<Fail>> = Mutex::new(None);
        let mut run = Run::new(&cfg, gen::scenario, |s: Scenario| -> TestResult {
            bump(&CASES);
            match eval_scenario(&s) {
                Eval::Pass => TestResult::Pass,
                Eval::Discard => TestResult::discard("setup failed"),
                Eval::Fail(f) => {
                    *last.lock().unwrap() = Some(f.clone());
                    raise(&f)
                },
            }
        });
        let mut failed = None;
        while run.wants_more() && Instant::now() < deadline {
            let Some(input) = run.draw() else { continue };
            let tested = run.test(&input);
            if tested.failed() {
                failed = Some(input.value.clone());
                break;
            }
        }
        drop(run);
        let (Some(value), Some(f)) = (failed, last.lock().unwrap().take()) else { continue };
        bump(&FAILURES);
        // One line per failure (unshrunk), for looking at the distribution of failures.
        if let Ok(mut log) = std::fs::OpenOptions::new().create(true).append(true).open(out.join("failures.tsv")) {
            let _ = writeln!(log, "{}\t{}\t{}", f.sig, f.spec, f.msg.replace(['\n', '\t'], " "));
        }
        if sig_count(&out, &f.sig) >= 3 || std::env::var_os("GIXFUZZ_TALLY_ONLY").is_some() {
            continue;
        }
        let small = shrink(&value, &f.sig, deadline);
        let f = match eval_scenario(&small) {
            Eval::Fail(g) if g.sig == f.sig => g,
            _ => f,
        };
        let case = minimal_case(&f);
        write_finding(&out, &f.sig, &f.msg, run_seed, &case);
        write_stats(&out, &base);
    }
    write_stats(&out, &base);
    let _ = std::fs::remove_file(out.join("current.json"));
}

// ---------------------------------------------------------------- check / eval

fn load_case(path: &str) -> Option<(Case, String)> {
    let text = std::fs::read_to_string(path).ok()?;
    Case::from_json(&serde_json::from_str(&text).ok()?)
}

/// Child side of `check`: prints `SIG <signature> <message>` or `PASS` or `UNDECIDED`.
fn eval_cmd(path: &str) -> i32 {
    install_panic_hook();
    std::panic::set_hook(Box::new(|info| {
        let at = info.location().map(|l| l.file().to_string());
        PANIC_AT.with(|p| *p.borrow_mut() = at);
    }));
    let Some((case, _)) = load_case(path) else {
        println!("UNDECIDED bad case file");
        return 2;
    };
    match eval_case(&case) {
        Eval::Pass => println!("PASS"),
        Eval::Discard => println!("UNDECIDED setup failed"),
        Eval::Fail(f) => println!("SIG {} {}", f.sig, f.msg),
    }
    0
}

fn check_cmd(path: &str) -> i32 {
    let Some((_, want)) = load_case(path) else {
        eprintln!("cannot read {path}");
        return 2;
    };
    let mut child = match Command::new(std::env::current_exe().unwrap())
        .args(["eval", path])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return 2,
    };
    let start = Instant::now();
    let limit = GIX_TIMEOUT + Duration::from_secs(40);
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) if start.elapsed() > limit => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            },
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => return 2,
        }
    };
    let mut text = String::new();
    if let Some(mut so) = child.stdout.take() {
        use std::io::Read;
        let _ = so.read_to_string(&mut text);
    }
    let got = match status {
        None => "crash__timeout".to_string(),
        Some(st) if st.code().is_none() || st.code() == Some(134) => "crash__abort".to_string(),
        Some(_) => {
            let line = text.lines().next().unwrap_or("");
            if line == "PASS" {
                println!("does not reproduce ({want})");
                return 0;
            } else if let Some(rest) = line.strip_prefix("SIG ") {
                rest.split(' ').next().unwrap_or("").to_string()
            } else {
                println!("cannot decide: {line}");
                return 2;
            }
        },
    };
    println!("{}", text.trim());
    if got == want {
        println!("reproduces: {want}");
        1
    } else {
        println!("does not reproduce {want} (got {got})");
        0
    }
}

fn record_abort(out: &str) {
    let out = PathBuf::from(out);
    let Some((case, _)) = load_case(out.join("current.json").to_str().unwrap()) else { return };
    write_finding(&out, "crash__abort", "the search process died (abort / signal) while gix evaluated this case", 0, &case);
    let _ = std::fs::remove_file(out.join("current.json"));
}

fn sample(seed: u64, n: u64) {
    let (mut ok, mut err) = (0, 0);
    for i in 0..n {
        let mut src = ephyra::Source::new(seed.wrapping_add(i), 10);
        let s = gen::scenario(&mut src);
        let dir = tempdir();
        let script = gen::render_setup(&s);
        println!("===== scenario {i}\n{script}");
        if let Some(repo) = setup(&dir.0, &script) {
            let objects = all_objects(&repo, &dir.0);
            for sp in &s.specs {
                let spec = gen::render_spec(sp, &objects);
                let a = git_rev_parse(&repo, &dir.0, &spec);
                if matches!(a, Some(GitAns::Oid(_))) { ok += 1 } else { err += 1 }
                println!("  {spec:30} {:?}", a.map(|a| match a { GitAns::Oid(o) => o[..8].to_string(), GitAns::Err(e) => format!("ERR {}", e.lines().last().unwrap_or("")), GitAns::Unreliable => "UNRELIABLE".to_string() }));
            }
        }
    }
    println!("git resolved {ok}, rejected {err}");
}

/// Triage helper: `gixfuzz resolve <case.json> <spec>...` builds the case's repo and prints gix's
/// and git's answer for each spec (the case's own specs when none are given).
fn resolve(case_path: &str, extra: &[String]) {
    let Some((mut case, _)) = load_case(case_path) else {
        eprintln!("cannot read {case_path}");
        return;
    };
    if !extra.is_empty() && extra[0] != "-" {
        case.specs = extra.to_vec();
    }
    let dir = tempdir();
    let Some(repo) = setup(&dir.0, &case.setup) else {
        eprintln!("setup failed");
        return;
    };
    let gix = gix_answers(&repo, &case.specs, &case);
    for (spec, g) in case.specs.iter().zip(gix) {
        if let Some(q) = graph::parse(spec) {
            let run = |args: &[&str]| git(&repo, &dir.0, args, None);
            println!("{spec}\n  gix: {g:?}\n  git: {:?}", graph::git_answer(&run, &q));
            continue;
        }
        println!("{spec}\n  gix: {g:?}\n  git: {:?}", git_rev_parse(&repo, &dir.0, spec));
    }
    if std::env::var_os("GIXFUZZ_KEEP").is_some() {
        println!("repo kept at {}", repo.display());
        std::mem::forget(dir);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let code = match args.get(1).map(String::as_str) {
        Some("search") if args.len() == 5 => {
            search(args[2].parse().expect("seed"), args[3].parse().expect("seconds"), PathBuf::from(&args[4]));
            0
        },
        Some("check") if args.len() == 3 => check_cmd(&args[2]),
        Some("eval") if args.len() == 3 => eval_cmd(&args[2]),
        Some("record-abort") if args.len() == 3 => {
            record_abort(&args[2]);
            0
        },
        Some("resolve") if args.len() >= 4 => {
            resolve(&args[2], &args[3..]);
            0
        },
        Some("sample") if args.len() == 4 => {
            sample(args[2].parse().expect("seed"), args[3].parse().expect("n"));
            0
        },
        _ => {
            eprintln!("usage: gixfuzz search <seed> <seconds> <out> | check <case.json> | eval <case.json> | record-abort <out> | sample <seed> <n>");
            2
        },
    };
    std::process::exit(code);
}
