//! A batch of pairs past sqleq: the frontend (does it lower?), QED and
//! `sqleq-solver` (do they prove?), `sqleq-fuzz` (does it refute, on
//! DuckDB?). Each pair also carries what PostgreSQL said on the case's own
//! data, so the verdicts can be judged:
//!
//! - an equivalent pair (seed against a variant) that `sqleq-fuzz` refutes:
//!   sqleq claims non-equivalence (its counterexample decides whose bug);
//! - an equivalent pair that lowers and neither prover proves: a proof
//!   sqleq misses;
//! - a mutant PostgreSQL refutes that a prover proves: an unsound proof.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use crate::tools;

/// A pair to check.
pub struct Pair {
    pub name: String,
    pub file_text: String,
    /// A variant (true) or a mutant (false).
    pub equivalent: bool,
    /// PostgreSQL on the case's data: the results differ (None: not run or
    /// skipped).
    pub pg_refutes: Option<bool>,
}

/// What each tool said of a pair.
#[derive(Clone, Debug, Default)]
pub struct Row {
    pub name: String,
    pub equivalent: bool,
    pub pg_refutes: Option<bool>,
    pub file: PathBuf,
    /// Err: the frontend's first error line.
    pub lowered: Option<Result<(), String>>,
    pub qed: Option<bool>,
    /// QED ran past [`QED_LIMIT`] and was killed (counts as not proved).
    pub qed_timed_out: bool,
    /// Where QED panicked (`src/pipeline/normal.rs:655`), if it did: a
    /// crash, which it reports as "not provable".
    pub qed_panic: Option<String>,
    /// The pair holds a real constant QED reads inexactly (qed-solver/prover#13:
    /// it parses reals as f32), so a proof of it may be that bug.
    pub rounded_real: bool,
    pub solver: Option<String>,
    /// The first word of sqleq-fuzz's verdict (NOT-EQUIVALENT,
    /// NO-COUNTEREXAMPLE, ERROR..) and its whole output.
    pub fuzz: Option<(String, String)>,
}

impl Row {
    pub fn proved(&self) -> bool {
        self.qed == Some(true) || self.solver.as_deref() == Some("EQ")
    }

    pub fn fuzz_refutes(&self) -> bool {
        self.fuzz.as_ref().is_some_and(|f| f.0 == "NOT-EQUIVALENT")
    }

    /// The finding this row is, if any. Causes already reported are named
    /// as such (`known-..`), so a run's new findings stand apart.
    pub fn finding(&self) -> Option<&'static str> {
        let lowered = matches!(self.lowered, Some(Ok(())));
        let site = self.qed_panic.as_deref().unwrap_or("");
        let at = |lines: &[&str]| lines.iter().any(|l| site.ends_with(l));
        if self.equivalent && self.fuzz_refutes() {
            Some("claims-not-equivalent")
        } else if at(&["normal.rs:655", "normal.rs:658", "normal.rs:659"]) {
            // qed-solver/prover#13: real constants read as f32.
            Some("known-qed-13-panic")
        } else if at(&["normal.rs:440", "partial.rs:185", "normal.rs:534"]) {
            // qed-solver/prover#14 and sqleq's set-operation typing.
            Some("known-qed-14-panic")
        } else if self.qed_panic.is_some() {
            Some("qed-panic")
        } else if !self.equivalent && self.pg_refutes == Some(true) && self.proved() && self.rounded_real {
            Some("known-qed-13-unsound")
        } else if !self.equivalent && self.pg_refutes == Some(true) && self.proved() {
            Some("proves-refuted")
        } else if !self.equivalent && self.pg_refutes == Some(true) && self.fuzz.as_ref().is_some_and(|f| f.0 == "NO-COUNTEREXAMPLE") {
            // A witnessed difference sqleq-fuzz did not find.
            Some("fuzz-missed")
        } else if self.equivalent && lowered && !self.proved() {
            Some("unproved")
        } else {
            None
        }
    }
}

/// Run `sqleq-solver` on `jobs` (JSON lines, one job per pair), resuming on
/// the jobs without a verdict after it halts on a `HANG`, until every job
/// has one or a run makes no progress. Verdicts by job name.
pub fn solve_all(jobs: &str, dir: &Path) -> BTreeMap<String, String> {
    let name_of = |l: &str| serde_json::from_str::<serde_json::Value>(l).ok().and_then(|v| v["name"].as_str().map(String::from));
    let mut pending: Vec<&str> = jobs.lines().filter(|l| !l.trim().is_empty()).collect();
    let mut verdicts: BTreeMap<String, String> = BTreeMap::new();
    for round in 0.. {
        if pending.is_empty() {
            break;
        }
        let (jobs_file, results) = (dir.join(format!("jobs-{round}.jsonl")), dir.join(format!("solver-{round}.jsonl")));
        fs::write(&jobs_file, pending.join("\n") + "\n").unwrap();
        let _ = Command::new(tools::bin("sqleq-solver")).arg(&jobs_file).arg(&results).arg("--timeout-ms=20000").output();
        let before = verdicts.len();
        for l in fs::read_to_string(&results).unwrap_or_default().lines() {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(l) {
                verdicts.insert(v["name"].as_str().unwrap_or("").to_string(), v["verdict"].as_str().unwrap_or("").to_string());
            }
        }
        if verdicts.len() == before {
            break;
        }
        pending.retain(|l| name_of(l).is_some_and(|n| !verdicts.contains_key(&n)));
    }
    verdicts
}

/// How long QED may take on one pair.
pub const QED_LIMIT: std::time::Duration = std::time::Duration::from_secs(20);

/// Whether a pair file holds a real or numeric literal that QED's f32
/// reading changes (qed-solver/prover#13): NaN or an infinity, a value an
/// f32 does not hold exactly, or one whose fraction overflows i32.
pub fn has_rounded_real(text: &str) -> bool {
    let mut rest = text;
    while let Some(i) = rest.find("'::") {
        let before = &rest[..i];
        let after = &rest[i + 3..];
        rest = after;
        let ty_real = ["float4", "float8", "numeric"].iter().any(|t| after.starts_with(t));
        let Some(start) = before.rfind('\'') else { continue };
        let lit = &before[start + 1..];
        if !ty_real {
            continue;
        }
        match lit.parse::<f64>() {
            Err(_) => return true,
            Ok(x) if !x.is_finite() => return true,
            Ok(x) => {
                let f = x as f32;
                if f64::from(f) != x || (x != 0.0 && (x.abs() >= 2_147_483_647.0 || x.abs() < 1e-9)) {
                    return true;
                }
                // A decimal no binary float holds (0.1) is read as a nearby one.
                if lit.contains('.') && f64::from(f) != x {
                    return true;
                }
                if lit.contains('.') && !lit.trim_end_matches('0').ends_with('.') && x.fract() != 0.0 && (x * 1024.0).fract() != 0.0 {
                    return true;
                }
            },
        }
    }
    false
}

/// The `file:line` of the first panic in a QED stderr file.
fn panic_site(stderr: &Path) -> Option<String> {
    let text = fs::read_to_string(stderr).ok()?;
    let at = text.lines().find(|l| l.contains("panicked at"))?;
    let site = at.split("panicked at ").nth(1)?.split(' ').next()?.trim_end_matches(':');
    Some(site.rsplitn(2, ':').nth(1)?.to_string())
}

/// QED on one lowered pair, alone in `dir` (it takes a directory and has
/// no options), killed after [`QED_LIMIT`]; true when it was.
fn qed_one(ir: &Path, dir: &Path) -> bool {
    fs::create_dir_all(dir).unwrap();
    let _ = fs::copy(ir, dir.join(ir.file_name().unwrap()));
    let err = fs::File::create(dir.join("stderr.txt")).ok();
    let stderr = err.map_or(std::process::Stdio::null(), std::process::Stdio::from);
    // Its own process group: QED races z3 and cvc5 as children, which a
    // kill of QED alone leaves running forever (they were found hours later,
    // loading the machine).
    use std::os::unix::process::CommandExt;
    let Ok(mut child) = Command::new(tools::qed()).arg(dir).stdout(std::process::Stdio::null()).stderr(stderr).process_group(0).spawn() else { return false };
    let deadline = std::time::Instant::now() + QED_LIMIT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return false,
            Ok(None) if std::time::Instant::now() > deadline => {
                // The whole group: QED and its solvers.
                unsafe {
                    libc::killpg(child.id() as libc::pid_t, libc::SIGKILL);
                }
                let _ = child.kill();
                let _ = child.wait();
                return true;
            },
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(20)),
            Err(_) => return false,
        }
    }
}

/// Run every tool on `pairs`, files under `dir` (emptied first).
pub fn check(pairs: Vec<Pair>, dir: &Path, threads: usize) -> Vec<Row> {
    check_with(pairs, dir, threads, true)
}

/// [`check`], with or without `sqleq-fuzz`.
pub fn check_with(pairs: Vec<Pair>, dir: &Path, threads: usize, fuzz: bool) -> Vec<Row> {
    let _ = fs::remove_dir_all(dir);
    for d in ["pairs", "qed", "qed1", "jobs"] {
        fs::create_dir_all(dir.join(d)).unwrap();
    }
    let mut rows: Vec<Row> = pairs
        .into_iter()
        .map(|p| {
            let file = dir.join("pairs").join(format!("{}.sql", p.name));
            fs::write(&file, &p.file_text).unwrap();
            let rounded_real = has_rounded_real(&p.file_text);
            Row { name: p.name, equivalent: p.equivalent, pg_refutes: p.pg_refutes, file, rounded_real, ..Row::default() }
        })
        .collect();
    // Frontend and fuzzer per pair, on `threads` threads.
    let chunk = rows.len().div_ceil(threads.max(1)).max(1);
    std::thread::scope(|scope| {
        for part in rows.chunks_mut(chunk) {
            let dir = dir.to_path_buf();
            scope.spawn(move || {
                for r in part {
                    let ir = dir.join("qed").join(format!("{}.json", r.name));
                    r.lowered = Some(tools::lower(&r.file, &ir));
                    if matches!(r.lowered, Some(Ok(()))) {
                        let job = dir.join("jobs").join(format!("{}.job", r.name));
                        let _ = Command::new(tools::bin("sqleq-frontend")).args(["--sqlsolver", "--ir"]).arg(&ir).args(["--name", &r.name, "-o"]).arg(&job).output();
                        r.qed_timed_out = qed_one(&ir, &dir.join("qed1").join(&r.name));
                        r.qed_panic = panic_site(&dir.join("qed1").join(&r.name).join("stderr.txt"));
                        let result = dir.join("qed1").join(&r.name).join(format!("{}.result", r.name));
                        r.qed = fs::read_to_string(result).ok().and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()).and_then(|v| v["provable"].as_bool());
                    }
                    if !fuzz {
                        continue;
                    }
                    if let Ok(o) = Command::new(tools::bin("sqleq-fuzz")).arg("file").arg(&r.file).output() {
                        let text = String::from_utf8_lossy(&o.stdout).into_owned() + &String::from_utf8_lossy(&o.stderr);
                        let first = text.split_whitespace().next().unwrap_or("").split(':').next().unwrap_or("").to_string();
                        r.fuzz = Some((first, text));
                    }
                }
            });
        }
    });
    // sqleq-solver on every job, resumed after a HANG (it halts its own
    // process then, by design) until every job has a verdict.
    let mut jobs = String::new();
    for r in &rows {
        jobs += &fs::read_to_string(dir.join("jobs").join(format!("{}.job", r.name))).unwrap_or_default();
    }
    let verdicts = solve_all(&jobs, dir);
    for r in rows.iter_mut() {
        r.solver = verdicts.get(&r.name).cloned();
    }
    rows
}

/// A cross-tab of the verdicts, equivalent pairs against mutants.
pub fn report(rows: &[Row]) -> String {
    let (eq, mu): (Vec<&Row>, Vec<&Row>) = rows.iter().partition(|r| r.equivalent);
    let count = |rs: &[&Row], f: &dyn Fn(&Row) -> bool| rs.iter().filter(|r| f(r)).count();
    let mut out = format!("{} pairs: {} equivalent, {} mutants\n  {:34} {:>15} {:>15}\n", rows.len(), eq.len(), mu.len(), "", "equivalent", "mutants");
    let lines: [(&str, &dyn Fn(&Row) -> bool); 12] = [
        ("PostgreSQL refutes (case data)", &|r| r.pg_refutes == Some(true)),
        ("frontend lowers", &|r| matches!(r.lowered, Some(Ok(())))),
        ("QED proves", &|r| r.qed == Some(true)),
        ("QED timed out", &|r| r.qed_timed_out),
        ("QED panicked", &|r| r.qed_panic.is_some()),
        ("holds a real QED rounds (#13)", &|r| r.rounded_real),
        ("sqleq-solver proves", &|r| r.solver.as_deref() == Some("EQ")),
        ("either prover proves", &|r| r.proved()),
        ("sqleq-fuzz refutes", &|r| r.fuzz_refutes()),
        ("sqleq-fuzz no counterexample", &|r| r.fuzz.as_ref().is_some_and(|f| f.0 == "NO-COUNTEREXAMPLE")),
        ("sqleq-fuzz error", &|r| r.fuzz.as_ref().is_some_and(|f| f.0 != "NOT-EQUIVALENT" && f.0 != "NO-COUNTEREXAMPLE")),
        ("finding", &|r| r.finding().is_some()),
    ];
    for (label, f) in lines {
        out += &format!("  {label:34} {:>7} / {:<6} {:>7} / {:<6}\n", count(&eq, f), eq.len(), count(&mu, f), mu.len());
    }
    let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
    for r in rows {
        if let Some(k) = r.finding() {
            *kinds.entry(k).or_default() += 1;
        }
    }
    out += &format!("  findings by kind: {kinds:?}\n");
    out
}

/// Every verdict per pair, as a TSV.
pub fn tsv(rows: &[Row]) -> String {
    let mut s = String::from("pair\tequivalent\tpg_refutes\tlowered\tqed\tsolver\tfuzz\tfinding\tqed_panic\n");
    let o = |x: Option<String>| x.unwrap_or_else(|| "-".into());
    for r in rows {
        let lowered = r.lowered.as_ref().map(|l| match l {
            Ok(()) => "yes".to_string(),
            Err(e) => format!("no: {}", e.chars().take(80).collect::<String>().replace('\t', " ")),
        });
        s += &format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            r.name,
            r.equivalent,
            o(r.pg_refutes.map(|b| b.to_string())),
            o(lowered),
            o(r.qed.map(|b| b.to_string())),
            o(r.solver.clone()),
            o(r.fuzz.as_ref().map(|f| f.0.clone())),
            r.finding().unwrap_or("-"),
            o(r.qed_panic.clone())
        );
    }
    s
}

#[cfg(test)]
mod tests {
    use super::has_rounded_real;

    #[test]
    fn rounded_reals_are_recognized() {
        for t in ["'NaN'::numeric", "'-Infinity'::float8", "'16777217'::float8", "'0.1'::numeric", "'1e10'::float8", "'9.7e-28'::float8"] {
            assert!(has_rounded_real(&format!("x = {t}")), "{t}");
        }
        for t in ["'0.5'::float8", "'3'::numeric", "'-2.25'::numeric", "'7'::int4", "'0.1'::text"] {
            assert!(!has_rounded_real(&format!("x = {t}")), "{t}");
        }
    }
}
