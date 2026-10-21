// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The query-coverage score of ephyra-sqlite's oracle inputs: the SQLite
//! code (in scope: [`scoped_counters`]) that 10 inputs are expected to reach
//! under the oracles. Frozen: a synthesis round changing the generator is
//! measured by it and must not change it.
//!
//! After a warm-up, `--inputs N` fresh draws of `case_queries` (seeds fixed by
//! `--seed`, sizes cycling 0..=100) each run under the oracle its index picks
//! (as the runner's workers do), with a 10-second deadline. With `p_c` the
//! share of inputs that reach counter c, the score is `Σ_c 1 − (1 − p_c)^10`.
//! Also printed: the score by amalgamation file (the 15 largest), inputs per
//! second, timeouts and failures (a failure is a SQLite bug or a generator or
//! oracle mistake: either way a round must explain it).
//!
//! `cargo run --release --bin qscore -- [--seed S] [--inputs N] [--files OUT.tsv]`

use std::{
    collections::{BTreeMap, HashMap},
    fs,
    panic::{self, AssertUnwindSafe},
    time::{Duration, Instant},
};

use ephyra::{coverage, Gen, Source};
use ephyra_sqlite::{
    cases::Case,
    expr::Query,
    probes::case_queries,
    sqlancer::{self, Tally},
};
use sqlite_coverage::{mix, scoped_counters, warm_up};

type Oracle = fn(&Case, &[Query]) -> Result<Tally, String>;

const ORACLES: [(&str, Oracle); 7] = [
    ("norec", sqlancer::norec),
    ("tlp", sqlancer::tlp),
    ("plans", sqlancer::plans),
    ("order", sqlancer::order),
    ("setops", sqlancer::setops),
    ("cte", sqlancer::ctes),
    ("window", sqlancer::windows),
];

const BATCH: i32 = 10;

fn main() {
    coverage::suppress_default_dump();
    panic::set_hook(Box::new(|_| {}));
    let args: Vec<String> = std::env::args().collect();
    let value = |flag: &str| args.iter().position(|a| a == flag).map(|i| args[i + 1].clone());
    let seed: u64 = value("--seed").map_or(0, |s| s.parse().unwrap());
    let n: u64 = value("--inputs").map_or(3000, |s| s.parse().unwrap());
    let scope = scoped_counters();
    let gen = case_queries();
    warm_up();
    let start = Instant::now();
    let mut reached: HashMap<usize, u64> = HashMap::new();
    let (mut timeouts, mut failures, mut comparisons) = (0u64, 0u64, 0u64);
    let mut failed: BTreeMap<String, u64> = BTreeMap::new();
    for i in 0..n {
        let Ok(input) = panic::catch_unwind(|| gen.generate(&mut Source::new(mix(seed, i), (i % 101) as usize))) else {
            failures += 1;
            *failed.entry("generator panic".into()).or_default() += 1;
            continue;
        };
        let (name, oracle) = ORACLES[(i % ORACLES.len() as u64) as usize];
        sqlancer::set_deadline(Some(Instant::now() + Duration::from_secs(10)));
        let before = coverage::snapshot().expect("an instrumented SQLite");
        let verdict = panic::catch_unwind(AssertUnwindSafe(|| oracle(&input.0, &input.1)));
        let delta = before.delta(&coverage::snapshot().expect("an instrumented SQLite"));
        let late = sqlancer::past_deadline();
        sqlancer::set_deadline(None);
        if late {
            timeouts += 1;
            continue;
        }
        match verdict {
            Ok(Ok(t)) => comparisons += t.comparisons,
            Ok(Err(e)) => {
                failures += 1;
                *failed.entry(format!("{name}: {}", e.lines().next().unwrap_or(""))).or_default() += 1;
            },
            Err(_) => {
                failures += 1;
                *failed.entry(format!("{name}: panic")).or_default() += 1;
            },
        }
        for &(c, _) in delta.hits() {
            if scope.contains_key(&c) {
                *reached.entry(c).or_default() += 1;
            }
        }
    }
    let secs = start.elapsed().as_secs_f64();
    let expected = |h: u64| 1.0 - (1.0 - h as f64 / n as f64).powi(BATCH);
    let score: f64 = reached.values().map(|&h| expected(h)).sum();
    let mut by_file: BTreeMap<&str, (f64, usize)> = BTreeMap::new();
    for (&c, &h) in &reached {
        let e = by_file.entry(scope[&c]).or_default();
        e.0 += expected(h);
        e.1 += 1;
    }
    println!(
        "qscore {score:.1}, {n} inputs, seed {seed}, {secs:.1} s ({:.0} inputs/s), {} counters reached of {} in scope, {timeouts} timeouts, {failures} failures, {comparisons} comparisons",
        n as f64 / secs,
        reached.len(),
        scope.len()
    );
    let mut files: Vec<_> = by_file.iter().collect();
    files.sort_by(|a, b| b.1 .0.total_cmp(&a.1 .0));
    for (file, (s, k)) in files.iter().take(15) {
        println!("  {file:16} {s:8.1}  ({k} counters reached)");
    }
    for (why, k) in &failed {
        println!("  failure x{k}: {why}");
    }
    if let Some(out) = value("--files") {
        let text: String = files.iter().map(|(f, (s, k))| format!("{f}\t{s:.2}\t{k}\n")).collect();
        fs::write(out, format!("file\tscore\tcounters\n{text}")).unwrap();
    }
}
