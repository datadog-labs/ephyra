// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Where evaluated inputs go besides the report: every input the runner
//! runs (random, targeted, shrink candidates, the minimal failure) reaches
//! each sink open for the run as a [`TestCase`], and the finished
//! [`Report`] follows.
//!
//! The observation file
//! ([`Config::report.observations`](crate::config::ReportConfig::observations),
//! read by Tyche) is one sink. Others are installed once per process with
//! [`install`] and see every run in it, whichever way the run was started
//! (`#[ephyra::test]`, [`check`](crate::check), [`Run`](crate::Run)):
//!
//! ```
//! use std::sync::{Arc, Mutex};
//!
//! use ephyra::sink::{self, CaseStatus, RunInfo, RunSink, Sink, TestCase};
//! use ephyra::{Config, check, gens};
//!
//! /// Counts the passing inputs of the property named `counted`.
//! struct Passes(Arc<Mutex<u64>>);
//! struct Counting(Arc<Mutex<u64>>);
//!
//! impl Sink for Passes {
//!     fn open(&self, run: &RunInfo<'_>) -> Option<Box<dyn RunSink>> {
//!         (run.property == "counted").then(|| Box::new(Counting(self.0.clone())) as _)
//!     }
//! }
//!
//! impl RunSink for Counting {
//!     fn test_case(&mut self, case: &TestCase<'_>) {
//!         if case.status == CaseStatus::Passed {
//!             *self.0.lock().unwrap() += 1;
//!         }
//!     }
//! }
//!
//! let passes = Arc::new(Mutex::new(0));
//! sink::install(Passes(passes.clone()));
//! let report = check(&Config::default().tests(100).name("counted"), gens::int(0..=9), |_| true);
//! assert_eq!(*passes.lock().unwrap(), report.tests);
//! ```
//!
//! While any sink is open the run renders each input with `Debug`,
//! evaluates [`observe`](crate::observe) closures and, in instrumented
//! builds, records the lines each input executed, so a sink that does not
//! want a run should return `None` from [`Sink::open`].

use std::{
    collections::BTreeMap,
    sync::{
        PoisonError,
        RwLock,
    },
    time::Duration,
};

use crate::{
    report::Report,
    runner::labels::FeatureValue,
};

/// Makes a [`RunSink`] for each run it wants. Installed with [`install`].
pub trait Sink: Send + Sync {
    /// The sink for the run described by `run`, or `None` to leave it out.
    fn open(&self, run: &RunInfo<'_>) -> Option<Box<dyn RunSink>>;
}

/// Receives one run's test cases, then its report.
pub trait RunSink: Send {
    /// One evaluated input, in the order the runner ran them.
    fn test_case(&mut self, case: &TestCase<'_>);

    /// The run is over. An error becomes a warning in the report.
    fn finish(&mut self, report: &Report) -> Result<(), String> {
        let _ = report;
        Ok(())
    }
}

/// The run a sink is opened for.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct RunInfo<'a> {
    /// The property's name, as in [`Report::name`].
    pub property: &'a str,
    /// The run's seed.
    pub seed: u64,
}

/// How an input's run ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CaseStatus {
    /// The property held.
    Passed,
    /// The property returned false, failed or panicked.
    Failed,
    /// Discarded by the generator or the property.
    GaveUp,
}

impl CaseStatus {
    /// The OpenPBTStats name: `passed`, `failed` or `gave_up`.
    pub fn as_str(self) -> &'static str {
        match self {
            CaseStatus::Passed => "passed",
            CaseStatus::Failed => "failed",
            CaseStatus::GaveUp => "gave_up",
        }
    }
}

/// One evaluated input and what was recorded about it.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct TestCase<'a> {
    /// How the run ended.
    pub status: CaseStatus,
    /// The failure message or the discard reason; empty for a pass.
    pub status_reason: &'a str,
    /// The input's `Debug` rendering; empty for a generator discard.
    pub representation: &'a str,
    /// Where the input came from: `random`, `target`, `shrink`, `minimal`, ...
    pub origin: &'a str,
    /// [`label`](crate::label)s, in call order.
    pub labels: &'a [String],
    /// [`feature`](crate::feature)s, in call order.
    pub features: &'a [(String, FeatureValue)],
    /// [`cover`](crate::cover) labels and whether their condition held.
    pub covers: &'a [(String, bool)],
    /// The highest [`target`](crate::target) score, if any.
    pub score: Option<f64>,
    /// [`observe`](crate::observe) and [`measure`](crate::measure) values,
    /// in call order.
    pub observations: &'a [(String, FeatureValue)],
    /// The source lines the input executed, by file, in instrumented builds.
    pub coverage: Option<&'a BTreeMap<String, Vec<u32>>>,
    /// Time spent generating the input.
    pub generate: Duration,
    /// Time spent running the property on it.
    pub execute: Duration,
    /// The seed the input was generated from.
    pub seed: u64,
    /// The iteration it was generated at.
    pub iteration: u64,
    /// The size it was generated with.
    pub size: usize,
    /// For a shrink candidate, whether it reproduced the failure being shrunk.
    pub shrink_accepted: Option<bool>,
}

static SINKS: RwLock<Vec<Box<dyn Sink>>> = RwLock::new(Vec::new());

/// Add `sink` for every later run in this process.
pub fn install(sink: impl Sink + 'static) {
    SINKS.write().unwrap_or_else(PoisonError::into_inner).push(Box::new(sink));
}

/// The installed sinks that want `run`.
pub(crate) fn open(run: &RunInfo<'_>) -> Vec<Box<dyn RunSink>> {
    SINKS
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .filter_map(|s| s.open(run))
        .collect()
}

/// The OpenPBTStats observation file of one run
/// (see [`Config::report.observations`](crate::config::ReportConfig::observations)).
#[cfg(feature = "json")]
pub(crate) struct ObservationFile {
    dir: std::path::PathBuf,
    run_start: f64,
    property: String,
    lines: Vec<String>,
}

#[cfg(feature = "json")]
impl ObservationFile {
    pub(crate) fn new(dir: std::path::PathBuf, property: String) -> ObservationFile {
        let run_start = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0.0, |d| d.as_secs_f64());
        ObservationFile { dir, run_start, property, lines: Vec::new() }
    }

    /// Append the lines to `<dir>/<property>.jsonl`.
    fn write(&self) -> std::io::Result<()> {
        use std::io::Write as _;
        std::fs::create_dir_all(&self.dir)?;
        let path = self.dir.join(crate::regressions::file_name(&self.property));
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
        for line in &self.lines {
            writeln!(file, "{line}")?;
        }
        Ok(())
    }
}

#[cfg(feature = "json")]
impl RunSink for ObservationFile {
    fn test_case(&mut self, case: &TestCase<'_>) {
        use serde_json::json;
        let feature_json = |value: &FeatureValue| {
            match value {
                FeatureValue::Number(n) => json!(n),
                FeatureValue::Text(t) => json!(t),
            }
        };
        let mut features = serde_json::Map::new();
        for label in case.labels {
            features.insert(label.clone(), json!(""));
        }
        for (name, value) in case.features {
            features.insert(name.clone(), feature_json(value));
        }
        for (label, met) in case.covers {
            features.insert(format!("cover:{label}"), json!(met.to_string()));
        }
        if let Some(score) = case.score {
            features.insert("target".to_string(), json!(score));
        }
        let observations: serde_json::Map<_, _> = case
            .observations
            .iter()
            .map(|(name, value)| (name.clone(), feature_json(value)))
            .collect();
        let record = json!({
            "type": "test_case",
            "run_start": self.run_start,
            "property": self.property,
            "status": case.status.as_str(),
            "status_reason": case.status_reason,
            "representation": case.representation,
            "how_generated": case.origin,
            "features": features,
            "observations": observations,
            "coverage": case.coverage,
            "timing": {
                "generate:input": case.generate.as_secs_f64(),
                "execute:test": case.execute.as_secs_f64(),
            },
            "metadata": {
                "seed": case.seed,
                "iteration": case.iteration,
                "size": case.size,
                "origin": case.origin,
                "shrink_accepted": case.shrink_accepted,
            },
        });
        self.lines.push(record.to_string());
    }

    fn finish(&mut self, report: &Report) -> Result<(), String> {
        let record = serde_json::json!({
            "type": "info",
            "run_start": self.run_start,
            "property": self.property,
            "title": "ephyra report",
            "content": format!("```\n{report}```"),
        });
        self.lines.push(record.to_string());
        self.write().map_err(|e| {
            format!("could not write the observation file under {}: {e}", self.dir.display())
        })
    }
}
