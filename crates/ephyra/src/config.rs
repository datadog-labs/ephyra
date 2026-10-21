// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Run configuration.
//!
//! All fields are public so a test can use struct-update syntax, and every
//! scalar field has a chainable setter, named after the field (without the
//! `_ms` unit suffix for `budget` and `timeout`):
//!
//! ```
//! use ephyra::Config;
//!
//! let a = Config { tests: 1000, seed: Some(7), ..Config::default() };
//! let b = Config::default().tests(1000).seed(7);
//! assert_eq!(a, b);
//! ```
//!
//! # Environment overrides
//!
//! [`check`](crate::check) applies overrides from the environment on top of
//! the configuration it is given, so a run can be steered without editing
//! code. Precedence: code < `EPHYRA_CONFIG` file < `EPHYRA_*`
//! variables. Problems (unknown `EPHYRA_*` names, unparsable values) are
//! reported as `config_env` health warnings, never silently ignored.
//!
//! | Variable | Field |
//! |---|---|
//! | `EPHYRA_CONFIG` | path of a JSON file with any subset of fields (needs the `json` feature) |
//! | `EPHYRA_TESTS` | `tests` |
//! | `EPHYRA_SEED` | `seed` |
//! | `EPHYRA_MAX_SIZE` | `max_size` |
//! | `EPHYRA_MAX_DISCARD_RATIO` | `max_discard_ratio` |
//! | `EPHYRA_BUDGET_MS` | `budget_ms` |
//! | `EPHYRA_TIMEOUT_MS` | `timeout_ms` |
//! | `EPHYRA_FILTER_MAX_ATTEMPTS` | `filter_max_attempts` |
//! | `EPHYRA_NAME` | `name` |
//! | `EPHYRA_REPLAY` | `replay` (`seed:iteration` from a report) |
//! | `EPHYRA_SHRINK` | `shrink.enabled` (`0`/`1`, `false`/`true`) |
//! | `EPHYRA_SHRINK_MAX_STEPS` | `shrink.max_steps` |
//! | `EPHYRA_SHRINK_TIMEOUT_MS` | `shrink.timeout_ms` |
//! | `EPHYRA_TARGET_TESTS` | `target.tests` |
//! | `EPHYRA_TARGET_POOL_SIZE` | `target.pool_size` |
//! | `EPHYRA_COVERAGE` | `coverage.enabled` |
//! | `EPHYRA_COVERAGE_FILTER` | `coverage.filter` |
//! | `EPHYRA_COVERAGE_GUIDED` | `coverage.guided` |
//! | `EPHYRA_PROFRAW_DIR` | `coverage.profraw_dir` |
//! | `EPHYRA_MIN_COVER` | `coverage.min_cover` |
//! | `EPHYRA_COVERAGE_TARGETS` | `coverage.targets_file` |
//! | `EPHYRA_COVERAGE_HTML` | `coverage.html` |
//! | `EPHYRA_LOCATE` | `locate.enabled` |
//! | `EPHYRA_LOCATE_NEIGHBOURHOOD` | `locate.neighbourhood` |
//! | `EPHYRA_LOCATE_TOP` | `locate.top` |
//! | `EPHYRA_REGRESSIONS` | `regressions.save` and `regressions.replay` |
//! | `EPHYRA_REGRESSIONS_DIR` | `regressions.dir` |
//! | `EPHYRA_TRUNCATE_DEBUG_AT` | `report.truncate_debug_at` |
//! | `EPHYRA_REPORT_JSON` | `report.json_path` |
//! | `EPHYRA_SAMPLES` | `report.samples` |
//! | `EPHYRA_OBSERVATIONS` | `report.observations` |

use std::{
    fmt,
    path::PathBuf,
    str::FromStr,
    time::Duration,
};

/// Where an input can be regenerated from: the run seed and the generation
/// attempt within the run. Printed as `seed:iteration` in reports and
/// accepted by `EPHYRA_REPLAY`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
pub struct ReplayPoint {
    /// The run seed.
    pub seed: u64,
    /// Zero-based generation attempt.
    pub iteration: u64,
}

impl fmt::Display for ReplayPoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.seed, self.iteration)
    }
}

impl FromStr for ReplayPoint {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (seed, iteration) =
            s.trim().split_once(':').ok_or_else(|| "expected seed:iteration".to_string())?;
        Ok(ReplayPoint {
            seed: seed.trim().parse().map_err(|e| format!("bad seed: {e}"))?,
            iteration: iteration.trim().parse().map_err(|e| format!("bad iteration: {e}"))?,
        })
    }
}

/// Everything that controls a run of [`check`](crate::check).
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct Config {
    /// Number of passing tests required before the property is considered
    /// to hold. Discarded inputs do not count. Default 10 000.
    pub tests: u64,
    /// Stop the random phase after this many milliseconds even if fewer than
    /// `tests` inputs have passed; the report says so. Default none.
    pub budget_ms: Option<u64>,
    /// Seed for the run. `None` picks a fresh random seed; the seed actually
    /// used is always in the report.
    pub seed: Option<u64>,
    /// Upper bound of the size parameter generators see. At iteration `i`,
    /// the size is `floor(log2(i + 1))`, capped here. This spends twice as
    /// many generation attempts at each successive size. Default 100.
    pub max_size: usize,
    /// Give up once more than `max_discard_ratio * tests` inputs have been
    /// discarded. Default 10.
    pub max_discard_ratio: f64,
    /// Stop with `TimedOut` after this many milliseconds. Default none.
    pub timeout_ms: Option<u64>,
    /// Attempts a `filter` makes before discarding the input. Default 100.
    pub filter_max_attempts: usize,
    /// Name of the property, used as the regressions file name. Defaults to
    /// the current thread's name, which under `cargo test` is the test's
    /// path (`module::test_fn`).
    pub name: Option<String>,
    /// An input to regenerate and test first (see `EPHYRA_REPLAY`). The
    /// run continues with random testing if it does not fail.
    pub replay: Option<ReplayPoint>,
    /// Shrinking budget.
    pub shrink: ShrinkConfig,
    /// The targeted phase (used when the property calls [`target`](crate::target)).
    pub target: TargetConfig,
    /// Coverage collection (needs the `coverage` feature and an instrumented
    /// build).
    pub coverage: CoverageConfig,
    /// Fault localization on failure (needs coverage).
    pub locate: LocateConfig,
    /// Remembered failures.
    pub regressions: RegressionsConfig,
    /// Report rendering.
    pub report: ReportConfig,
}

/// Budget for shrinking a counterexample.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct ShrinkConfig {
    /// Whether to shrink at all. Default true.
    pub enabled: bool,
    /// Maximum number of shrink candidates to try. Default 10 000.
    pub max_steps: u64,
    /// Time budget for shrinking in milliseconds. Default 60 000.
    pub timeout_ms: Option<u64>,
}

/// Budget of the targeted phase, which runs after the random phase when
/// the property reported scores through [`target`](crate::target).
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct TargetConfig {
    /// Mutated inputs to try. `0` disables the targeted phase. Default 256.
    pub tests: u64,
    /// Best-scoring inputs kept as mutation seeds. Default 8.
    pub pool_size: usize,
    /// Mutations tried per seed before it is retired. Default 16.
    pub energy: u32,
    /// Share of iterations that draw a fresh random input instead of
    /// mutating a seed. Default 0.1.
    pub explore: f64,
}

/// Coverage collection and use, with the `coverage` feature on a binary
/// built with `-C instrument-coverage`. Without both, these settings do
/// nothing.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct CoverageConfig {
    /// Collect per-input coverage and summarise it in the report. Default
    /// true (a no-op when unavailable).
    pub enabled: bool,
    /// Track only functions whose demangled name starts with this crate or
    /// module path. Empty (the default) means the crate the property was
    /// defined in: `#[ephyra::test]` uses the package name, `check` reads
    /// it off the property's type name. Set it explicitly when the property
    /// lives in an integration-test crate and the code under test in the
    /// library (`"my_lib"`), or to a module (`"my_lib::parser"`).
    pub filter: String,
    /// Coverage-guided search: inputs that reach new counters join the
    /// targeted phase's pool, and the targeted phase runs even when the
    /// property reports no scores. Default false.
    pub guided: bool,
    /// Write `<dir>/<name>.profraw` at the end of the run for `llvm-cov`.
    /// Default none.
    pub profraw_dir: Option<PathBuf>,
    /// Minimum share of passing tests each `cover` label must reach; labels
    /// below it produce a `cover` health warning. Default 0.05.
    pub min_cover: f64,
    /// Source ranges the search should reach (see the `coverage` module
    /// docs). Their functions join the tracked set, hitting them counts as
    /// novelty for the guided search, and the report says whether each was
    /// reached. Default empty.
    pub targets: Vec<CoverageTarget>,
    /// A JSON-lines file of further targets, read when it exists (needs the
    /// `json` feature). Default `.ephyra/targets.jsonl`.
    pub targets_file: Option<PathBuf>,
    /// Directory to write `<name>.html` into: a self-contained coverage
    /// grid (modules, files, functions, lines) with the targets and samples,
    /// from which a range can be turned into a new target. Default none.
    pub html: Option<PathBuf>,
}

/// A source range for coverage-guided search to reach. `file` is matched
/// by suffix against the paths the compiler recorded, so `src/lib.rs`
/// works from any working directory; lines are inclusive.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
pub struct CoverageTarget {
    /// The source file.
    pub file: String,
    /// First line, inclusive.
    pub begin_line: u32,
    /// Last line, inclusive.
    pub end_line: u32,
}

impl CoverageTarget {
    /// A target for `file` and the inclusive line range.
    pub fn new(file: impl Into<String>, lines: std::ops::RangeInclusive<u32>) -> Self {
        CoverageTarget { file: file.into(), begin_line: *lines.start(), end_line: *lines.end() }
    }
}

impl fmt::Display for CoverageTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.begin_line == self.end_line {
            write!(f, "{}:{}", self.file, self.begin_line)
        } else {
            write!(f, "{}:{}-{}", self.file, self.begin_line, self.end_line)
        }
    }
}

/// Read targets from a JSON-lines file: either `{"file", "begin_line",
/// "end_line"}` objects or Antithesis's
/// `{"antithesis_targeted_coverage": {"locations": [...]}}`. A missing
/// file is no targets.
#[cfg(feature = "json")]
pub fn load_targets(path: &std::path::Path) -> Result<Vec<CoverageTarget>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let mut targets = Vec::new();
    for (number, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(line)
            .map_err(|e| format!("{}:{}: {e}", path.display(), number + 1))?;
        let locations = match value.get("antithesis_targeted_coverage") {
            Some(wrapped) => {
                wrapped.get("locations").and_then(|l| l.as_array()).cloned().unwrap_or_default()
            },
            None => vec![value],
        };
        for location in locations {
            let target: CoverageTarget = serde_json::from_value(location)
                .map_err(|e| format!("{}:{}: {e}", path.display(), number + 1))?;
            targets.push(target);
        }
    }
    Ok(targets)
}

impl Default for CoverageConfig {
    fn default() -> Self {
        CoverageConfig {
            enabled: true,
            filter: String::new(),
            guided: false,
            profraw_dir: None,
            min_cover: 0.05,
            targets: Vec::new(),
            targets_file: Some(PathBuf::from(".ephyra/targets.jsonl")),
            html: None,
        }
    }
}

/// Spectrum-based fault localization: when a property fails, the coverage
/// of every run (passing inputs, the shrink trajectory and a mutant
/// neighbourhood of the shrunk input) ranks the functions of the crate
/// under test by how much they separate failing runs from passing ones.
/// Needs the `coverage` feature and an instrumented build.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "json", serde(default))]
pub struct LocateConfig {
    /// Localize failures. Default true (a no-op when coverage is not
    /// collected).
    pub enabled: bool,
    /// How many mutants of the shrunk input to run for the spectrum.
    /// Default 64.
    pub neighbourhood: usize,
    /// How many suspects to report. Default 10.
    pub top: usize,
}

impl Default for LocateConfig {
    fn default() -> Self {
        LocateConfig { enabled: true, neighbourhood: 64, top: 10 }
    }
}

/// Where and whether failing inputs are remembered (needs the `json`
/// feature; without it both flags are ignored).
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct RegressionsConfig {
    /// Directory holding one `<name>.jsonl` per property. Default
    /// `.ephyra/regressions`, relative to the working directory (the
    /// crate root under `cargo test`).
    pub dir: PathBuf,
    /// Append the shrunk failing input on failure. Default true.
    pub save: bool,
    /// Replay stored entries before random testing. Default true.
    pub replay: bool,
}

/// How reports are rendered.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct ReportConfig {
    /// `Debug` renderings of counterexamples longer than this many characters
    /// are truncated (the report says so). Default 2 000.
    pub truncate_debug_at: usize,
    /// Append each report as one JSON line to this file (needs the `json`
    /// feature). Default none.
    pub json_path: Option<PathBuf>,
    /// How many passing inputs to show in the report: the smallest and the
    /// largest by size, and a random sample of the rest. Default 5.
    pub samples: usize,
    /// Directory to append one observation file per property to, in the
    /// OpenPBTStats JSON-lines format that
    /// [Tyche](https://github.com/tyche-pbt/tyche-extension) visualises:
    /// one `test_case` line per evaluated input with its status,
    /// representation, origin, features, custom [`observe`](crate::observe)
    /// values, timing and, in instrumented builds, the source lines it
    /// executed, followed by an `info` line carrying the report. Renders every
    /// input, so it is opt-in. Needs the `json` feature. Default none. The
    /// file is one [sink](crate::sink); others can be installed alongside it.
    pub observations: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            tests: 10_000,
            budget_ms: None,
            seed: None,
            max_size: 100,
            max_discard_ratio: 10.0,
            timeout_ms: None,
            filter_max_attempts: crate::source::DEFAULT_FILTER_MAX_ATTEMPTS,
            name: None,
            replay: None,
            shrink: ShrinkConfig::default(),
            target: TargetConfig::default(),
            coverage: CoverageConfig::default(),
            locate: LocateConfig::default(),
            regressions: RegressionsConfig::default(),
            report: ReportConfig::default(),
        }
    }
}

impl Default for ShrinkConfig {
    fn default() -> Self {
        ShrinkConfig { enabled: true, max_steps: 10_000, timeout_ms: Some(60_000) }
    }
}

impl ShrinkConfig {
    /// Whether shrinking that began at `started` and took `steps` is over budget.
    pub(crate) fn exhausted(&self, steps: u64, started: std::time::Instant) -> bool {
        steps >= self.max_steps
            || self.timeout_ms.is_some_and(|ms| started.elapsed() >= Duration::from_millis(ms))
    }
}

impl Default for TargetConfig {
    fn default() -> Self {
        TargetConfig { tests: 256, pool_size: 8, energy: 16, explore: 0.1 }
    }
}

impl Default for RegressionsConfig {
    fn default() -> Self {
        RegressionsConfig { dir: PathBuf::from(".ephyra/regressions"), save: true, replay: true }
    }
}

impl Default for ReportConfig {
    fn default() -> Self {
        ReportConfig { truncate_debug_at: 2_000, json_path: None, samples: 5, observations: None }
    }
}

impl Config {
    /// Set [`tests`](Self::tests).
    pub fn tests(mut self, tests: u64) -> Self {
        self.tests = tests;
        self
    }

    /// Set [`budget_ms`](Self::budget_ms) from a `Duration`.
    pub fn budget(mut self, budget: Duration) -> Self {
        self.budget_ms = Some(budget.as_millis() as u64);
        self
    }

    /// Set [`seed`](Self::seed).
    pub fn seed(mut self, seed: u64) -> Self {
        self.seed = Some(seed);
        self
    }

    /// Set [`max_size`](Self::max_size).
    pub fn max_size(mut self, max_size: usize) -> Self {
        self.max_size = max_size;
        self
    }

    /// Set [`max_discard_ratio`](Self::max_discard_ratio).
    pub fn max_discard_ratio(mut self, ratio: f64) -> Self {
        self.max_discard_ratio = ratio;
        self
    }

    /// Set [`timeout_ms`](Self::timeout_ms) from a `Duration`.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout_ms = Some(timeout.as_millis() as u64);
        self
    }

    /// Set [`filter_max_attempts`](Self::filter_max_attempts).
    pub fn filter_max_attempts(mut self, attempts: usize) -> Self {
        self.filter_max_attempts = attempts.max(1);
        self
    }

    /// Set [`name`](Self::name).
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set [`replay`](Self::replay).
    pub fn replay(mut self, point: ReplayPoint) -> Self {
        self.replay = Some(point);
        self
    }

    /// Disable shrinking.
    pub fn no_shrink(mut self) -> Self {
        self.shrink.enabled = false;
        self
    }

    /// Neither save nor replay regressions.
    pub fn no_regressions(mut self) -> Self {
        self.regressions.save = false;
        self.regressions.replay = false;
        self
    }

    /// Set [`regressions.dir`](RegressionsConfig::dir).
    pub fn regressions_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.regressions.dir = dir.into();
        self
    }

    /// The run timeout as a `Duration`, if any.
    pub fn timeout_duration(&self) -> Option<Duration> {
        self.timeout_ms.map(Duration::from_millis)
    }

    /// Parse a configuration from JSON. Missing fields take their defaults.
    #[cfg(feature = "json")]
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }


    /// Apply `EPHYRA_*` overrides from `vars`, returning the new
    /// configuration and a description of every problem encountered.
    pub fn apply_env_vars(
        mut self,
        vars: impl IntoIterator<Item = (String, String)>,
    ) -> (Self, Vec<String>) {
        let mut problems = Vec::new();
        let mut vars: Vec<(String, String)> =
            vars.into_iter().filter(|(k, _)| k.starts_with("EPHYRA_")).collect();
        vars.sort();

        // The config file comes first so that individual variables win.
        if let Some((_, path)) = vars.iter().find(|(k, _)| k == "EPHYRA_CONFIG") {
            #[cfg(feature = "json")]
            match std::fs::read_to_string(path) {
                Ok(text) => {
                    match Self::from_json(&text) {
                        Ok(from_file) => self = from_file,
                        Err(e) => problems.push(format!("EPHYRA_CONFIG={path}: invalid JSON: {e}")),
                    }
                },
                Err(e) => problems.push(format!("EPHYRA_CONFIG={path}: cannot read: {e}")),
            }
            #[cfg(not(feature = "json"))]
            problems
                .push(format!("EPHYRA_CONFIG={path}: ignored, this build has no `json` feature"));
        }

        for (key, value) in &vars {
            // Apply the parsed value, or report what was expected instead.
            macro_rules! set {
                (int, |$x:ident| $apply:expr) => {
                    set!(value.parse().ok(), "an integer", |$x| $apply)
                };
                (number, |$x:ident| $apply:expr) => {
                    set!(value.parse().ok(), "a number", |$x| $apply)
                };
                (bool, |$x:ident| $apply:expr) => {
                    set!(parse_bool(value), "0/1/true/false", |$x| $apply)
                };
                ($parsed:expr, $what:literal, |$x:ident| $apply:expr) => {
                    match $parsed {
                        Some($x) => $apply,
                        None => {
                            problems.push(format!("{key}={value}: expected {}, ignored", $what))
                        },
                    }
                };
            }
            match key.as_str() {
                "EPHYRA_CONFIG" => {},
                "EPHYRA_TESTS" => set!(int, |n| self.tests = n),
                "EPHYRA_SEED" => set!(int, |n| self.seed = Some(n)),
                "EPHYRA_MAX_SIZE" => set!(int, |n| self.max_size = n),
                "EPHYRA_MAX_DISCARD_RATIO" => set!(number, |n| self.max_discard_ratio = n),
                "EPHYRA_BUDGET_MS" => set!(int, |n| self.budget_ms = Some(n)),
                "EPHYRA_TIMEOUT_MS" => set!(int, |n| self.timeout_ms = Some(n)),
                "EPHYRA_FILTER_MAX_ATTEMPTS" => {
                    set!(value.parse::<usize>().ok(), "an integer", |n| {
                        self.filter_max_attempts = n.max(1)
                    })
                },
                "EPHYRA_NAME" => self.name = Some(value.clone()),
                "EPHYRA_REPLAY" => {
                    set!(value.parse().ok(), "seed:iteration", |p| self.replay = Some(p))
                },
                "EPHYRA_SHRINK" => set!(bool, |b| self.shrink.enabled = b),
                "EPHYRA_SHRINK_MAX_STEPS" => set!(int, |n| self.shrink.max_steps = n),
                "EPHYRA_SHRINK_TIMEOUT_MS" => set!(int, |n| self.shrink.timeout_ms = Some(n)),
                "EPHYRA_TARGET_TESTS" => set!(int, |n| self.target.tests = n),
                "EPHYRA_TARGET_POOL_SIZE" => set!(int, |n| self.target.pool_size = n),
                "EPHYRA_COVERAGE" => set!(bool, |b| self.coverage.enabled = b),
                "EPHYRA_COVERAGE_FILTER" => self.coverage.filter = value.clone(),
                "EPHYRA_COVERAGE_GUIDED" => set!(bool, |b| self.coverage.guided = b),
                "EPHYRA_PROFRAW_DIR" => self.coverage.profraw_dir = Some(PathBuf::from(value)),
                "EPHYRA_COVERAGE_TARGETS" => {
                    self.coverage.targets_file = Some(PathBuf::from(value))
                },
                "EPHYRA_COVERAGE_HTML" => self.coverage.html = Some(PathBuf::from(value)),
                "EPHYRA_MIN_COVER" => set!(number, |n| self.coverage.min_cover = n),
                "EPHYRA_LOCATE" => set!(bool, |b| self.locate.enabled = b),
                "EPHYRA_LOCATE_NEIGHBOURHOOD" => set!(int, |n| self.locate.neighbourhood = n),
                "EPHYRA_LOCATE_TOP" => set!(int, |n| self.locate.top = n),
                "EPHYRA_REGRESSIONS" => {
                    set!(bool, |b| {
                        self.regressions.save = b;
                        self.regressions.replay = b;
                    })
                },
                "EPHYRA_REGRESSIONS_DIR" => self.regressions.dir = PathBuf::from(value),
                "EPHYRA_TRUNCATE_DEBUG_AT" => set!(int, |n| self.report.truncate_debug_at = n),
                "EPHYRA_REPORT_JSON" => self.report.json_path = Some(PathBuf::from(value)),
                "EPHYRA_OBSERVATIONS" => self.report.observations = Some(PathBuf::from(value)),
                "EPHYRA_SAMPLES" => set!(int, |n| self.report.samples = n),
                other => {
                    problems.push(format!(
                        "{other} is not a ephyra setting (see ephyra::config); ignored"
                    ))
                },
            }
        }
        (self, problems)
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn setters_match_fields() {
        let config = Config::default()
            .tests(5)
            .seed(9)
            .max_size(3)
            .timeout(Duration::from_millis(250))
            .no_shrink()
            .no_regressions();
        assert_eq!(config.tests, 5);
        assert_eq!(config.seed, Some(9));
        assert_eq!(config.max_size, 3);
        assert_eq!(config.timeout_duration(), Some(Duration::from_millis(250)));
        assert!(!config.shrink.enabled);
        assert!(!config.regressions.save && !config.regressions.replay);
    }

    #[test]
    fn env_overrides_and_problems() {
        let (config, problems) = Config::default().apply_env_vars(vars(&[
            ("EPHYRA_TESTS", "12"),
            ("EPHYRA_SEED", "42"),
            ("EPHYRA_SHRINK", "off"),
            ("EPHYRA_REGRESSIONS", "0"),
            ("EPHYRA_REPLAY", " 42:7 "),
            ("EPHYRA_MAX_SIZE", "lots"),
            ("EPHYRA_TEST", "1"),
            ("PATH", "/bin"),
        ]));
        assert_eq!(config.tests, 12);
        assert_eq!(config.seed, Some(42));
        assert!(!config.shrink.enabled);
        assert!(!config.regressions.save);
        assert_eq!(config.replay, Some(ReplayPoint { seed: 42, iteration: 7 }));
        assert_eq!(config.max_size, 100);
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems[0].contains("EPHYRA_MAX_SIZE=lots"));
        assert!(problems[1].contains("EPHYRA_TEST is not a ephyra setting"));
    }

    #[cfg(feature = "json")]
    #[test]
    fn config_file_then_variables() {
        let dir = std::env::temp_dir().join(format!("ephyra-config-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cfg.json");
        std::fs::write(&path, r#"{ "tests": 7, "max_size": 3 }"#).unwrap();
        let (config, problems) = Config::default().apply_env_vars(vars(&[
            ("EPHYRA_CONFIG", path.to_str().unwrap()),
            ("EPHYRA_TESTS", "9"),
        ]));
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(config.tests, 9, "variables beat the file");
        assert_eq!(config.max_size, 3, "the file beats the defaults");
        let (_, problems) =
            Config::default().apply_env_vars(vars(&[("EPHYRA_CONFIG", "/nonexistent/cfg.json")]));
        assert_eq!(problems.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(feature = "json")]
    #[test]
    fn json_round_trip_with_defaults() {
        let config = Config::from_json(r#"{ "tests": 12, "shrink": { "max_steps": 3 } }"#).unwrap();
        assert_eq!(config.tests, 12);
        assert_eq!(config.shrink.max_steps, 3);
        assert_eq!(config.max_size, Config::default().max_size);
        let again = Config::from_json(&serde_json::to_string(&config).unwrap()).unwrap();
        assert_eq!(again, config);
    }
}

#[cfg(all(test, feature = "json"))]
mod target_tests {
    use super::*;

    #[test]
    fn targets_files_accept_both_shapes_and_may_be_missing() {
        let dir = std::env::temp_dir().join(format!("ephyra-targets-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("targets.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"file\": \"src/lib.rs\", \"begin_line\": 10, \"end_line\": 12}\n",
                "\n",
                "{\"antithesis_targeted_coverage\": {\"locations\": [",
                "{\"file\": \"core/foo.rs\", \"begin_line\": 3, \"end_line\": 3},",
                "{\"file\": \"core/bar.rs\", \"begin_line\": 7, \"end_line\": 9}]}}\n"
            ),
        )
        .unwrap();
        let targets = load_targets(&path).unwrap();
        assert_eq!(
            targets,
            vec![
                CoverageTarget::new("src/lib.rs", 10..=12),
                CoverageTarget::new("core/foo.rs", 3..=3),
                CoverageTarget::new("core/bar.rs", 7..=9),
            ]
        );
        assert_eq!(targets[0].to_string(), "src/lib.rs:10-12");
        assert_eq!(targets[1].to_string(), "core/foo.rs:3");
        assert_eq!(load_targets(&dir.join("missing.jsonl")).unwrap(), vec![]);
        std::fs::write(&path, "{\"file\": 1}\n").unwrap();
        assert!(load_targets(&path).unwrap_err().contains("targets.jsonl:1"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
