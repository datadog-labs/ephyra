// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The outcome of a run: [`Report`] and its parts.
//!
//! A report is both human-readable (`Display`) and, with the default `json`
//! feature, machine-readable (`Report::to_json`). Everything an author or
//! an agent needs to act on a failure is in it: the seed, the replay point,
//! the failing input before and after shrinking, the panic message and
//! location, timing, and the distribution statistics collected while the
//! property held.

use std::{
    collections::BTreeMap,
    fmt,
    path::PathBuf,
    time::Duration,
};

mod render;

#[cfg(feature = "coverage")]
pub(crate) use self::render::joined;
use crate::config::ReplayPoint;

/// How a run ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "json", derive(serde::Serialize))]
pub enum Status {
    /// The required number of tests passed (or the time budget ran out
    /// with every test passing).
    Passed,
    /// A counterexample was found. See [`Report::failure`].
    Failed,
    /// Too many inputs were discarded. See [`Config::max_discard_ratio`](crate::Config::max_discard_ratio).
    GaveUp,
    /// The run exceeded [`Config::timeout_ms`](crate::Config::timeout_ms).
    TimedOut,
    /// The generator itself panicked. See [`Report::abort`].
    Aborted,
}

/// A source location captured from a panic.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "json", derive(serde::Serialize))]
pub struct Location {
    /// File path as reported by the panic.
    pub file: String,
    /// 1-based line.
    pub line: u32,
    /// 1-based column.
    pub column: u32,
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}", self.file, self.line, self.column)
    }
}

/// What kind of failure the property produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "json", derive(serde::Serialize))]
pub enum FailureKind {
    /// The property returned `false`.
    False,
    /// The property returned an error or an explicit failure message.
    Error,
    /// The property panicked.
    Panic,
}

/// One failing input, rendered with `Debug`.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "json", derive(serde::Serialize))]
pub struct Counterexample {
    /// `Debug` rendering of the input, possibly truncated.
    pub debug: String,
    /// Whether `debug` was cut short (see `Config::report.truncate_debug_at`).
    pub truncated: bool,
}

/// What shrinking did.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "json", derive(serde::Serialize))]
pub struct ShrinkStats {
    /// Candidates tried.
    pub steps: u64,
    /// Candidates accepted.
    pub accepted: u64,
    /// Candidates skipped because they rendered longer than the current best.
    pub rejected_not_smaller: u64,
    /// Candidates skipped because they had been seen before.
    pub rejected_seen: u64,
    /// Whether the step or time budget stopped shrinking early.
    pub budget_exhausted: bool,
    /// Time spent shrinking.
    pub duration: Duration,
}

/// A counterexample and the failure it triggered.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "json", derive(serde::Serialize))]
pub struct Failure {
    /// What kind of failure.
    pub kind: FailureKind,
    /// The error, failure message, or panic message (of the shrunk input
    /// when shrinking ran).
    pub message: Option<String>,
    /// Where a panic originated, when known.
    pub location: Option<Location>,
    /// The input that first failed.
    pub original: Counterexample,
    /// The size parameter the original input was generated at.
    pub size: usize,
    /// Zero-based index of the generation attempt that produced the
    /// original input.
    pub iteration: u64,
    /// The shrunk input, when shrinking ran and made progress.
    pub shrunk: Option<Counterexample>,
    /// Where the original input can be regenerated from
    /// (`EPHYRA_REPLAY=seed:iteration`). `None` when the input came from
    /// the targeted phase; then the whole run must be repeated with
    /// [`Report::seed`].
    pub replay: Option<ReplayPoint>,
    /// Whether the property failed differently when re-run on the same
    /// input. A flaky property cannot be shrunk reliably.
    pub flaky: bool,
    /// What shrinking did, when it ran.
    pub shrink: Option<ShrinkStats>,
}

impl Failure {
    /// The best counterexample available: the shrunk one if any, else the
    /// original.
    pub fn counterexample(&self) -> &Counterexample {
        self.shrunk.as_ref().unwrap_or(&self.original)
    }
}

/// One of the distinct failures of a run that looks for several
/// ([`Config::failures`](crate::Config::failures) above 1), in the order
/// they were found: the first is also [`Report::failure`].
///
/// ```
/// use ephyra::{check, gens, Config};
///
/// let config = Config::default().seed(1).failures(5).no_regressions();
/// let report = check(&config, gens::int(0u32..=100), |x| {
///     assert!(x % 3 != 1, "one more than a multiple of three");
///     assert!(x % 5 != 1, "one more than a multiple of five");
/// });
/// assert_eq!(report.failures.len(), 2);
/// for found in &report.failures {
///     println!("{} x{}: {}", found.signature, found.count, found.failure.counterexample().debug);
/// }
/// ```
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "json", derive(serde::Serialize))]
pub struct DistinctFailure {
    /// What the failures in this group share: the name given with
    /// [`signature`](crate::signature), else the one from
    /// [`Config::signature`](crate::Config::signature), else the panic
    /// location, `returned false`, or the error message with its numbers
    /// and quoted text blanked.
    pub signature: String,
    /// Failing inputs with this signature (shrink candidates not counted).
    pub count: u64,
    /// The first input that failed this way, shrunk while keeping the
    /// signature, with its replay point.
    pub failure: Failure,
}

/// What a [`Config::signature`](crate::Config::signature) function sees of
/// a failure, to name it.
///
/// ```
/// use ephyra::{report::FailureKind, Config};
///
/// // Name panics by their message's first word, other failures by kind.
/// let config = Config::default().failures(5).signature(|failure| {
///     match (failure.kind, &failure.message) {
///         (FailureKind::Panic, Some(message)) => {
///             message.split_whitespace().next().unwrap_or("").to_string()
///         },
///         (kind, _) => format!("{kind:?}"),
///     }
/// });
/// assert!(config.signature.is_some());
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FailureInfo {
    /// What kind of failure.
    pub kind: FailureKind,
    /// The error, failure message or panic message.
    pub message: Option<String>,
    /// Where a panic originated, when known.
    pub location: Option<Location>,
    /// `Debug` rendering of the failing input, cut at
    /// `Config::report.truncate_debug_at` characters.
    pub value: String,
}

/// A generator panic.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "json", derive(serde::Serialize))]
pub struct Abort {
    /// The panic message.
    pub message: String,
    /// Where the panic originated, when known.
    pub location: Option<Location>,
    /// Zero-based index of the generation attempt.
    pub iteration: u64,
}

/// Something about the run that an author should look at.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "json", derive(serde::Serialize))]
pub struct HealthWarning {
    /// Stable identifier, e.g. `flaky`, `discard_ratio`, `config_env`.
    pub kind: String,
    /// Human-readable explanation with the numbers behind it.
    pub message: String,
}

/// What happened to the explicit replay point (`EPHYRA_REPLAY`).
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "json", derive(serde::Serialize))]
pub struct ReplayOutcome {
    /// The point that was replayed.
    pub point: ReplayPoint,
    /// Whether the property failed on it.
    pub reproduced: bool,
}

/// What happened with the regressions file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "json", derive(serde::Serialize))]
pub struct RegressionsSummary {
    /// The file used, when regressions were enabled.
    pub file: Option<PathBuf>,
    /// Stored entries replayed before random testing.
    pub replayed: u64,
    /// Entries whose regenerated input no longer matched the stored one
    /// (the generator changed).
    pub stale: u64,
    /// Whether this run's failure was appended.
    pub saved: bool,
}

/// What the targeted phase achieved (present when the property called
/// [`target`](crate::target)).
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "json", derive(serde::Serialize))]
pub struct TargetSummary {
    /// Whether the property reported `target` scores; when false the pool
    /// was seeded by coverage novelty alone and `best_score` is 0.
    pub scored: bool,
    /// Mutated inputs evaluated in the targeted phase.
    pub iterations: u64,
    /// Times the best score improved.
    pub improvements: u64,
    /// The highest score observed in the whole run.
    pub best_score: f64,
    /// `Debug` rendering of the input that reached `best_score`.
    pub best_debug: String,
}

/// Causality analysis of a failing trace (see
/// [`loops::causality`](crate::loops::causality)): how likely the failure
/// was to happen again when the trace was rewound to a prefix and continued
/// with fresh randomness, at each prefix measured.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "json", derive(serde::Serialize))]
pub struct Causality {
    /// Steps in the failing trace.
    pub steps: usize,
    /// The prefixes measured, shortest first.
    pub points: Vec<CausalPoint>,
    /// The largest rise in the failure rate between neighbouring points,
    /// when the rate rose at all.
    pub jump: Option<CausalJump>,
}

/// One prefix of a [`Causality`] analysis.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "json", derive(serde::Serialize))]
pub struct CausalPoint {
    /// Steps of the failing trace kept before continuing.
    pub prefix: usize,
    /// Continuations tested (discarded ones are not counted).
    pub branches: u64,
    /// Continuations that failed the way the trace did.
    pub failures: u64,
}

impl CausalPoint {
    /// The share of continuations that failed; 0 when none were tested.
    pub fn rate(&self) -> f64 {
        if self.branches == 0 { 0.0 } else { self.failures as f64 / self.branches as f64 }
    }
}

/// Where the failure rate of a [`Causality`] analysis rose the most: from
/// `before` after `from` steps to `after` after `to` steps. When `to` is
/// `from + 1`, step `to` (counting from 1) is where the failure became
/// likely.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "json", derive(serde::Serialize))]
pub struct CausalJump {
    /// The shorter prefix.
    pub from: usize,
    /// The longer prefix.
    pub to: usize,
    /// The failure rate after `from` steps.
    pub before: f64,
    /// The failure rate after `to` steps.
    pub after: f64,
}

/// How a run was started, read from the environment cargo gives a test or
/// example when it runs it: enough for `ephyra visualize` to run the same
/// property again with `cargo test -p <package> --tests -- <test> --exact`
/// or `cargo run -p <package> --example <target>`.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
pub struct RunSpec {
    /// The package (`CARGO_PKG_NAME`).
    pub package: Option<String>,
    /// `test` for a test binary (unit or integration tests), `example` for
    /// an example.
    pub kind: String,
    /// The binary's name: the example's name or the test crate's name.
    pub target: Option<String>,
    /// The test's path inside the binary (`module::test_fn`), when the run
    /// happened under the test harness.
    pub test: Option<String>,
}

impl RunSpec {
    /// Read from the running process; `None` outside cargo.
    pub fn current() -> Option<RunSpec> {
        let package = std::env::var("CARGO_PKG_NAME").ok()?;
        let exe = std::env::current_exe().ok()?;
        let parent = exe.parent().and_then(|p| p.file_name()).and_then(|n| n.to_str())?;
        let stem = exe.file_stem()?.to_str()?;
        // `deps/reports-3f2a…` carries a metadata hash; `examples/bst` does not.
        let name = match stem.rsplit_once('-') {
            Some((name, hash))
                if hash.len() == 16 && hash.bytes().all(|b| b.is_ascii_hexdigit()) =>
            {
                name
            },
            _ => stem,
        };
        let kind = if parent == "examples" { "example" } else { "test" };
        let test = (kind == "test")
            .then(|| std::thread::current().name().map(str::to_string))
            .flatten()
            .filter(|t| t != "main" && !t.is_empty());
        Some(RunSpec {
            package: Some(package),
            kind: kind.to_string(),
            target: Some(name.to_string()),
            test,
        })
    }
}

/// The outcome of a run of [`check`](crate::check).
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "json", derive(serde::Serialize))]
pub struct Report {
    /// The property's name (`Config::name`, else the test's name).
    pub name: String,
    /// How the run ended.
    pub status: Status,
    /// The seed the run used. Pass it to `Config::seed` to reproduce the
    /// whole run.
    pub seed: u64,
    /// Passing tests.
    pub tests: u64,
    /// Passing tests with distinct inputs (by `Debug` rendering).
    pub unique_tests: u64,
    /// Discarded inputs (from generators and from the property).
    pub discarded: u64,
    /// Generation attempts made (passing, discarded and failing).
    pub iterations: u64,
    /// Whether the random phase ended because `Config::budget_ms` ran out
    /// before `Config::tests` inputs had passed.
    pub stopped_by_budget: bool,
    /// The `max_size` the run used.
    pub max_size: usize,
    /// Wall-clock time of the whole run.
    pub duration: Duration,
    /// Time spent generating inputs.
    pub gen_time: Duration,
    /// Time spent running the property.
    pub prop_time: Duration,
    /// Share of generated (non-discarded) inputs that were distinct.
    pub distinct_inputs_ratio: f64,
    /// Distinct inputs seen (y) after each number of generated inputs (x),
    /// recorded whenever a new input appeared; a curve that flattens early
    /// means the generator repeats itself. From the main thread only.
    pub distinct_growth: Series,
    /// Numeric features recorded with [`feature`](crate::feature), summarised
    /// over the passing tests.
    pub features: BTreeMap<String, FeatureSummary>,
    /// Per-input timing, when any input ran.
    pub timing: Option<Timing>,
    /// A few of the passing inputs (the smallest and the largest by size,
    /// and a random sample), so the report shows what the generator makes.
    pub samples: Vec<Sample>,
    /// The failure, when `status` is `Failed`; the first one found when the
    /// run looked for several.
    pub failure: Option<Failure>,
    /// Every distinct failure, the first included, when
    /// [`Config::failures`](crate::Config::failures) is above 1; empty
    /// otherwise.
    #[cfg_attr(feature = "json", serde(skip_serializing_if = "Vec::is_empty"))]
    pub failures: Vec<DistinctFailure>,
    /// The generator panic, when `status` is `Aborted`.
    pub abort: Option<Abort>,
    /// What happened to the explicit replay point, if one was given.
    pub replay: Option<ReplayOutcome>,
    /// What happened with the regressions file.
    pub regressions: RegressionsSummary,
    /// The targeted phase, when the property reported scores.
    pub target: Option<TargetSummary>,
    /// Causality analysis of the failure, when a loop ran it.
    #[cfg_attr(feature = "json", serde(skip_serializing_if = "Option::is_none"))]
    pub causality: Option<Causality>,
    /// Coverage of the crate under test, when collected (`coverage` feature,
    /// instrumented build).
    #[cfg(feature = "coverage")]
    #[cfg_attr(feature = "json", serde(skip_serializing_if = "Option::is_none"))]
    pub coverage: Option<crate::coverage::Summary>,
    /// Fault localization for a failure: the functions ranked by how much
    /// their coverage separates failing runs from passing ones (`coverage`
    /// feature, instrumented build).
    #[cfg(feature = "coverage")]
    #[cfg_attr(feature = "json", serde(skip_serializing_if = "Option::is_none"))]
    pub locate: Option<crate::coverage::Locate>,
    /// Path of the profraw written for `llvm-cov`, when requested.
    pub profraw: Option<PathBuf>,
    /// Path of the coverage grid page, when `Config::coverage.html` is set.
    pub coverage_html: Option<PathBuf>,
    /// How this run was started, when it ran under cargo, so a tool such as
    /// `ephyra visualize` can run the same property again.
    #[cfg_attr(feature = "json", serde(skip_serializing_if = "Option::is_none"))]
    pub run: Option<RunSpec>,
    /// Labels registered with [`cover`](crate::cover) and the share of passing
    /// tests that carried each.
    pub covers: BTreeMap<String, f64>,
    /// How many passing tests carried each label (see [`label`](crate::label)).
    pub labels: BTreeMap<String, u64>,
    /// How many passing tests ran at each size.
    pub sizes: BTreeMap<usize, u64>,
    /// How many inputs were discarded for each reason.
    pub discard_reasons: BTreeMap<String, u64>,
    /// Things worth looking at.
    pub health: Vec<HealthWarning>,
}

impl Report {
    pub(crate) fn new(seed: u64, max_size: usize) -> Self {
        Report {
            name: String::new(),
            status: Status::Passed,
            seed,
            tests: 0,
            unique_tests: 0,
            discarded: 0,
            iterations: 0,
            stopped_by_budget: false,
            max_size,
            duration: Duration::ZERO,
            gen_time: Duration::ZERO,
            prop_time: Duration::ZERO,
            distinct_inputs_ratio: 1.0,
            distinct_growth: Series::default(),
            features: BTreeMap::new(),
            timing: None,
            samples: Vec::new(),
            failure: None,
            failures: Vec::new(),
            abort: None,
            replay: None,
            regressions: RegressionsSummary::default(),
            target: None,
            causality: None,
            #[cfg(feature = "coverage")]
            coverage: None,
            #[cfg(feature = "coverage")]
            locate: None,
            profraw: None,
            coverage_html: None,
            run: None,
            covers: BTreeMap::new(),
            labels: BTreeMap::new(),
            sizes: BTreeMap::new(),
            discard_reasons: BTreeMap::new(),
            health: Vec::new(),
        }
    }

    /// Whether the property held.
    pub fn passed(&self) -> bool {
        self.status == Status::Passed
    }

    /// The best counterexample, if the run failed.
    pub fn counterexample(&self) -> Option<&Counterexample> {
        self.failure.as_ref().map(Failure::counterexample)
    }

    /// Panic with the rendered report unless the property held. Use this at
    /// the end of a `#[test]`.
    #[track_caller]
    pub fn assert_passed(&self) {
        if !self.passed() {
            panic!("{self}");
        }
    }

    /// Render the report as pretty-printed JSON.
    #[cfg(feature = "json")]
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("Report is always serialisable")
    }

    pub(crate) fn warn(&mut self, kind: &str, message: impl Into<String>) {
        self.health.push(HealthWarning { kind: kind.to_string(), message: message.into() });
    }
}

/// A time series sampled whenever the value grew, thinned to at most 512
/// points (every other point is dropped when it fills up, so the resolution
/// stays even across the run).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "json", derive(serde::Serialize))]
pub struct Series {
    /// `(x, y)` points in increasing `x`.
    pub points: Vec<(u64, u64)>,
}

impl Series {
    /// Append a point, thinning the series when it fills up.
    pub(crate) fn push(&mut self, x: u64, y: u64) {
        self.points.push((x, y));
        if self.points.len() > 512 {
            let last = self.points.pop();
            let mut keep = false;
            self.points.retain(|_| {
                keep = !keep;
                keep
            });
            self.points.extend(last);
        }
    }
}

/// One bar of a feature histogram: values in `from..=to`.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "json", derive(serde::Serialize))]
pub struct Bucket {
    /// Lower bound, inclusive.
    pub from: f64,
    /// Upper bound, inclusive.
    pub to: f64,
    /// Values in the bucket.
    pub count: u64,
}

/// A numeric feature over the passing tests (see [`feature`](crate::feature)).
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "json", derive(serde::Serialize))]
pub struct FeatureSummary {
    /// Values recorded.
    pub count: u64,
    /// Smallest value.
    pub min: f64,
    /// Largest value.
    pub max: f64,
    /// Arithmetic mean.
    pub mean: f64,
    /// Median.
    pub median: f64,
    /// 90th percentile.
    pub p90: f64,
    /// Up to eight buckets between `min` and `max`; one per value when the
    /// values are a few integers.
    pub histogram: Vec<Bucket>,
}

impl FeatureSummary {
    /// Summarise finite values.
    pub(crate) fn of(mut values: Vec<f64>) -> FeatureSummary {
        values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let count = values.len() as u64;
        let min = values[0];
        let max = values[values.len() - 1];
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        let median = values[values.len() / 2];
        let p90 = values[(values.len() * 9 / 10).min(values.len() - 1)];
        let integral = values.iter().all(|v| v.fract() == 0.0);
        let mut histogram = Vec::new();
        if integral && max - min < 8.0 {
            let mut at = min;
            while at <= max {
                let n = values.iter().filter(|&&v| v == at).count() as u64;
                histogram.push(Bucket { from: at, to: at, count: n });
                at += 1.0;
            }
        } else if max > min {
            let width = if integral { ((max - min + 1.0) / 8.0).ceil() } else { (max - min) / 8.0 };
            for i in 0..8 {
                let from = min + width * i as f64;
                let to = if integral { from + width - 1.0 } else { from + width };
                let last = i == 7;
                let n = values
                    .iter()
                    .filter(|&&v| v >= from && (v < from + width || (last && v <= max)))
                    .count() as u64;
                histogram.push(Bucket { from, to: to.min(max), count: n });
            }
        } else {
            histogram.push(Bucket { from: min, to: max, count });
        }
        FeatureSummary { count, min, max, mean, median, p90, histogram }
    }
}

/// Quantiles of a set of durations.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "json", derive(serde::Serialize))]
pub struct DurationStats {
    /// Median.
    pub median: Duration,
    /// 90th percentile.
    pub p90: Duration,
    /// Longest.
    pub max: Duration,
}

impl DurationStats {
    /// From per-input microseconds; `None` when empty.
    pub(crate) fn of(mut micros: Vec<u32>) -> Option<DurationStats> {
        if micros.is_empty() {
            return None;
        }
        micros.sort_unstable();
        let at = |q: usize| Duration::from_micros(u64::from(micros[q.min(micros.len() - 1)]));
        Some(DurationStats {
            median: at(micros.len() / 2),
            p90: at(micros.len() * 9 / 10),
            max: at(micros.len() - 1),
        })
    }
}

/// Per-input timing over the run.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "json", derive(serde::Serialize))]
pub struct Timing {
    /// Time the property took per input.
    pub prop: DurationStats,
    /// Time generation took per input (mutation, in the targeted phase).
    pub generation: DurationStats,
    /// `Debug` rendering of the input the property took longest on, when
    /// known.
    pub slowest: Option<String>,
}

/// Where an input came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "json", derive(serde::Serialize))]
#[cfg_attr(feature = "json", serde(rename_all = "lowercase"))]
pub enum Origin {
    /// The random phase.
    Random,
    /// A fresh input drawn during the targeted phase.
    Explore,
    /// A mutant of a pool input in the targeted phase.
    Mutation,
    /// The explicit replay point.
    Replay,
    /// A failing trace resumed after a prefix, in causality analysis.
    Branch,
}

impl Origin {
    /// The lowercase name used in reports and observation files.
    pub fn as_str(self) -> &'static str {
        match self {
            Origin::Random => "random",
            Origin::Explore => "explore",
            Origin::Mutation => "mutation",
            Origin::Replay => "replay",
            Origin::Branch => "branch",
        }
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One of the passing inputs shown in the report.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "json", derive(serde::Serialize))]
pub struct Sample {
    /// `Debug` rendering, cut at 200 characters with a trailing `…`.
    pub debug: String,
    /// How the input was produced.
    pub origin: Origin,
    /// The size it was generated at.
    pub size: usize,
    /// Its iteration number.
    pub iteration: u64,
}
