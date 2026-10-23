// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! A probe of the type-affinity and collation gap (synthesis round
//! `round/affinity-collate`; not the frozen score): over inputs drawn and
//! run as `qscore` runs them (seeds `mix(seed, i)`, sizes cycling 0..=100,
//! the oracle by index, a 10-second deadline), the SQLite functions of
//! affinity and collation: per function, counters reached of all, and the
//! expected reach of 10 inputs (`Σ 1 − (1 − p_c)^10`, as qscore scores);
//! then the share of inputs holding each feature of
//! `ephyra_sqlite::probes::gap_features`. An input whose drawing panics is
//! skipped and counted (shares and reaches stay over all `N`).
//!
//! `cargo run --release --bin gapcov -- [--seed S] [--inputs N]`

use std::{
    collections::{BTreeMap, HashMap},
    panic::{self, AssertUnwindSafe},
    time::{Duration, Instant},
};

use ephyra::{
    coverage::{self, Functions},
    Gen, Source,
};
use ephyra_sqlite::{
    cases::Case,
    expr::Query,
    probes::{case_queries, gap_features},
    sqlancer::{self, Tally},
};
use sqlite_coverage::{mix, warm_up};

type Oracle = fn(&Case, &[Query]) -> Result<Tally, String>;

const ORACLES: [Oracle; 7] = [sqlancer::norec, sqlancer::tlp, sqlancer::plans, sqlancer::order, sqlancer::setops, sqlancer::ctes, sqlancer::windows];

/// Affinity: declared types, applying and comparing under affinities,
/// numbers read from text and rendered as text. Collation: finding and
/// choosing collating sequences, the built-in ones, comparing under them.
const FUNCTIONS: &[&str] = &[
    "sqlite3AffinityType",
    "sqlite3ColumnSetColl",
    "sqlite3AddCollateType",
    "sqlite3TableAffinity",
    "sqlite3IndexAffinityStr",
    "sqlite3ExprAffinity",
    "sqlite3CompareAffinity",
    "comparisonAffinity",
    "sqlite3IndexAffinityOk",
    "sqlite3ExprNeedsNoAffinityChange",
    "applyAffinity",
    "applyNumericAffinity",
    "alsoAnInt",
    "sqlite3VdbeMemNumerify",
    "sqlite3VdbeIntegerAffinity",
    "sqlite3VdbeMemStringify",
    "vdbeMemRenderNum",
    "sqlite3AtoF",
    "sqlite3MemRealValueRC",
    "sqlite3ValueApplyAffinity",
    "sqlite3ExprCollSeq",
    "sqlite3ExprNNCollSeq",
    "sqlite3BinaryCompareCollSeq",
    "sqlite3ExprCompareCollSeq",
    "sqlite3ExprCollSeqMatch",
    "sqlite3LocateCollSeq",
    "sqlite3FindCollSeq",
    "sqlite3GetCollSeq",
    "sqlite3CheckCollSeq",
    "sqlite3ColumnColl",
    "sqlite3ExprAddCollateToken",
    "sqlite3ExprAddCollateString",
    "sqlite3ExprSkipCollate",
    "sqlite3ExprSkipCollateAndLikely",
    "multiSelectCollSeq",
    "sqlite3KeyInfoFromExprList",
    "nocaseCollatingFunc",
    "rtrimCollFunc",
    "binCollFunc",
    "sqlite3StrNICmp",
    "vdbeCompareMemString",
    "sqlite3MemCompare",
    "sqlite3VdbeRecordCompareWithSkip",
    "vdbeRecordCompareString",
    "sqlite3IsBinary",
];

fn main() {
    coverage::suppress_default_dump();
    panic::set_hook(Box::new(|_| {}));
    let args: Vec<String> = std::env::args().collect();
    let value = |flag: &str| args.iter().position(|a| a == flag).map(|i| args[i + 1].clone());
    let seed: u64 = value("--seed").map_or(0, |s| s.parse().unwrap());
    let n: u64 = value("--inputs").map_or(3000, |s| s.parse().unwrap());
    let mut owner: HashMap<usize, &str> = HashMap::new();
    let mut total: BTreeMap<&str, usize> = BTreeMap::new();
    for f in Functions::get().all() {
        let name = f.name.rsplit(':').next().unwrap_or(&f.name);
        if let Some(&want) = FUNCTIONS.iter().find(|w| **w == name) {
            for c in f.counters.clone() {
                owner.insert(c, want);
            }
            *total.entry(want).or_default() += f.counters.len();
        }
    }
    let gen = case_queries();
    warm_up();
    let start = Instant::now();
    let mut reached: HashMap<usize, u64> = HashMap::new();
    let mut features: BTreeMap<&str, u64> = BTreeMap::new();
    let (mut failures, mut panics) = (0, 0);
    for i in 0..n {
        // A generator panic (silent under the hook above) skips the input,
        // counted, as qscore counts it.
        let Ok(input) = panic::catch_unwind(|| gen.generate(&mut Source::new(mix(seed, i), (i % 101) as usize))) else {
            panics += 1;
            continue;
        };
        for f in gap_features(&input.0, &input.1) {
            *features.entry(f).or_default() += 1;
        }
        let oracle = ORACLES[(i % ORACLES.len() as u64) as usize];
        sqlancer::set_deadline(Some(Instant::now() + Duration::from_secs(10)));
        let before = coverage::snapshot().expect("an instrumented SQLite");
        let verdict = panic::catch_unwind(AssertUnwindSafe(|| oracle(&input.0, &input.1)));
        let delta = before.delta(&coverage::snapshot().expect("an instrumented SQLite"));
        let late = sqlancer::past_deadline();
        sqlancer::set_deadline(None);
        if late {
            continue;
        }
        failures += u64::from(!matches!(verdict, Ok(Ok(_))));
        for &(c, _) in delta.hits() {
            if owner.contains_key(&c) {
                *reached.entry(c).or_default() += 1;
            }
        }
    }
    let expected = |h: u64| 1.0 - (1.0 - h as f64 / n as f64).powi(10);
    let mut by: BTreeMap<&str, (usize, f64)> = BTreeMap::new();
    for (&c, &h) in &reached {
        let e = by.entry(owner[&c]).or_default();
        e.0 += 1;
        e.1 += expected(h);
    }
    println!("gapcov, {n} inputs, seed {seed}, {:.1} s, {failures} failures, {panics} generator panics", start.elapsed().as_secs_f64());
    let (mut all_r, mut all_t, mut all_s) = (0, 0, 0.0);
    for f in FUNCTIONS {
        let Some(&t) = total.get(f) else {
            println!("  {f:36} (not in this build)");
            continue;
        };
        let (r, s) = by.get(f).copied().unwrap_or_default();
        println!("  {f:36} {r:4} / {t:4} counters, expected {s:7.2}");
        all_r += r;
        all_t += t;
        all_s += s;
    }
    println!("  {:36} {all_r:4} / {all_t:4} counters, expected {all_s:7.2}", "all");
    for (f, k) in &features {
        println!("  share {f:24} {:6.2}%", 100.0 * *k as f64 / n as f64);
    }
}
