// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The test loop: [`check`] and [`quickcheck`].

mod coverage;
mod exec;
pub mod labels;
pub(crate) mod panic;
mod remember;
mod search;
mod shrinking;
mod stats;
pub mod testable;

use std::{
    collections::{
        BTreeMap,
        HashSet,
    },
    fmt::Debug,
    time::Instant,
};

use {
    rand_chacha::ChaCha8Rng,
    rand_core::SeedableRng,
};

#[cfg(feature = "coverage")]
pub(crate) use self::coverage::crate_of;
pub use self::{
    exec::{
        Input,
        Tested,
    },
    testable::{
        Outcome,
        TestResult,
        Testable,
    },
};
use crate::{
    Arbitrary,
    Config,
    Gen,
    any,
    health,
    pool::Pool,
    report::{
        Report,
        Status,
    },
    source::{
        mix,
        random_seed,
    },
};

use self::{
    coverage::CoverageState,
    exec::{
        Exec,
        Signature,
        Skipped,
    },
    shrinking::{
        Found,
        Shrinking,
    },
    stats::{
        Stats,
        micros,
    },
};
pub(crate) use self::{
    exec::evaluate,
    stats::{
        SAMPLE_CHARS,
        cap,
    },
};

/// Run `property` against inputs from `generator` until [`Config::tests`]
/// inputs pass, one fails, too many are discarded, or a time limit elapses.
///
/// Before random testing, an explicit replay point ([`Config::replay`] or
/// `EPHYRA_REPLAY`) and any stored regressions for this property are
/// regenerated and tested. A failing input is shrunk with the generator's
/// shrinker (see [`Config::shrink`]), remembered in the regressions file,
/// and reported with the seed and iteration that regenerate it.
/// Environment overrides (`EPHYRA_*`, see [`config`](crate::config)) are
/// applied on top of `config`.
///
/// If the property reports scores through [`target`](crate::target), the
/// random phase is followed by a targeted phase that mutates the
/// best-scoring inputs with the generator's mutator (see [`Config::target`]).
///
/// ```
/// use ephyra::{check, gens, Config};
///
/// let report = check(&Config::default().seed(1).tests(200), gens::vec(gens::int(-100i32..=100)), |mut v| {
///     v.sort();
///     v.windows(2).all(|w| w[0] <= w[1])
/// });
/// report.assert_passed();
/// ```
pub fn check<G, R, P>(config: &Config, generator: G, property: P) -> Report
where
    G: Gen,
    G::Value: Clone + Debug,
    R: Testable,
    P: FnMut(G::Value) -> R,
{
    let mut run = Run::new(config, generator, property);
    if run.remembered() {
        return run.finish();
    }
    run.random_search();
    if run.pending.is_none() && run.report.status == Status::Passed {
        run.targeted_search();
    }
    run.finish()
}

/// [`check`] with the type's [`Arbitrary`] generator and the default
/// [`Config`].
///
/// ```
/// use ephyra::quickcheck;
///
/// quickcheck(|(a, b): (u8, u8)| u16::from(a) + u16::from(b) >= u16::from(a)).assert_passed();
/// ```
pub fn quickcheck<T, R, P>(property: P) -> Report
where
    T: Arbitrary + Clone + Debug,
    R: Testable,
    P: FnMut(T) -> R,
{
    check(&Config::default(), any::<T>(), property)
}

fn property_name(config: &Config) -> String {
    config
        .name
        .clone()
        .or_else(|| std::thread::current().name().map(str::to_string))
        .unwrap_or_else(|| "unnamed".to_string())
}

/// One property-based test as a sequence of steps, for testing loops
/// written by hand. [`check`] is this loop:
///
/// ```
/// use ephyra::{Config, Run, gens};
///
/// let config = Config::default().seed(1).tests(200);
/// let mut run = Run::new(&config, gens::vec(gens::int(-100i32..=100)), |v: Vec<i32>| {
///     v.iter().map(|&x| i64::from(x)).sum::<i64>().abs() < 1_000_000
/// });
/// while run.wants_more() {
///     let Some(input) = run.draw() else { continue };
///     let tested = run.test(&input);
///     if tested.passed() {
///         run.keep(input.value, &tested);
///     }
/// }
/// let report = run.finish();
/// report.assert_passed();
/// ```
///
/// [`new`](Self::new) applies the environment overrides, chooses the seed
/// and starts coverage collection when configured. [`draw`](Self::draw)
/// makes a random input, [`mutate`](Self::mutate) a variant of one, and
/// [`test`](Self::test) runs the property on any input, recording the
/// verdict, labels, features, timing and coverage in the report. Passing
/// inputs may be [kept](Self::keep) as seeds for mutation and taken back
/// with [`pool_pick`](Self::pool_pick). [`wants_more`](Self::wants_more)
/// applies the stopping rules (the number of tests, the time budget, the
/// timeout, the discard limit, and a failure). Shrinking is a step too:
/// [`shrink`](Self::shrink) runs the built-in driver on the failure
/// whenever the loop likes, or [`reproduces`](Self::reproduces) and
/// [`shrunk_to`](Self::shrunk_to) let the loop drive its own.
/// [`finish`](Self::finish) shrinks the first failure if nothing did,
/// localizes it, remembers it and writes the report, its files and its
/// pages.
///
/// A loop is reproducible by seed: all randomness comes from the run, so
/// the same seed replays the same run. Inputs from `draw` also carry a
/// [replay point](Input::replay) that regenerates them alone; mutants do
/// not, so a failure found by mutation is remembered by its rendering.
pub struct Run<G: Gen, P> {
    config: Config,
    generator: G,
    exec: Exec<P>,
    /// Also the run's seed and property name.
    report: Report,
    started: Instant,
    seen: HashSet<u64>,
    generated: u64,
    pool: Pool<G::Value>,
    /// `cover` labels: (times met, times seen) over passing tests.
    covers: BTreeMap<String, (u64, u64)>,
    stats: Stats,
    /// Whether any passing input reported a `target` score.
    saw_score: bool,
    /// The targeted phase's choices: explore or mutate, and which seed.
    rng: ChaCha8Rng,
    /// The first failure, shrunk and reported by [`finish`](Self::finish).
    pending: Option<Found<G::Value>>,
    /// Mutations made through [`mutate`](Self::mutate), for the target
    /// summary of hand-written loops.
    mutations: u64,
    /// The pending failure's shrinking, once it was reproduced.
    shrinking: Option<Shrinking<G::Value>>,
    /// Held while coverage is collected, so runs do not interleave.
    #[cfg(feature = "coverage")]
    lease: Option<crate::coverage::Lease>,
}

impl<G, R, P> Run<G, P>
where
    G: Gen,
    G::Value: Clone + Debug,
    R: Testable,
    P: FnMut(G::Value) -> R,
{
    /// Set up a run: the environment overrides applied on top of `config`,
    /// the seed chosen (or taken from the config), coverage collection
    /// started when configured and the binary is instrumented, and the
    /// report begun.
    pub fn new(config: &Config, generator: G, property: P) -> Self {
        let (config, problems) = config.clone().apply_env_vars(std::env::vars());
        let seed = config.seed.unwrap_or_else(random_seed);
        let mut report = Report {
            name: property_name(&config),
            run: crate::report::RunSpec::current(),
            ..Report::new(seed, config.max_size)
        };
        for problem in problems {
            report.warn("config_env", problem);
        }
        let mut sinks: Vec<Box<dyn crate::sink::RunSink>> = Vec::new();
        #[cfg(feature = "json")]
        if let Some(dir) = &config.report.observations {
            sinks.push(Box::new(crate::sink::ObservationFile::new(
                dir.clone(),
                report.name.clone(),
            )));
        }
        #[cfg(not(feature = "json"))]
        if config.report.observations.is_some() {
            report.warn("observations", "observation files need the `json` feature");
        }
        sinks.extend(crate::sink::open(&crate::sink::RunInfo { property: &report.name, seed }));
        #[cfg_attr(not(feature = "coverage"), allow(unused_mut))]
        let mut run = Run {
            pool: Pool::new(config.target.pool_size, config.target.energy),
            stats: Stats::new(config.report.samples),
            config,
            generator,
            exec: Exec { property, coverage: CoverageState::default(), sinks },
            report,
            started: Instant::now(),
            seen: HashSet::new(),
            generated: 0,
            covers: BTreeMap::new(),
            saw_score: false,
            rng: ChaCha8Rng::seed_from_u64(mix(seed, u64::MAX - 1)),
            pending: None,
            mutations: 0,
            shrinking: None,
            #[cfg(feature = "coverage")]
            lease: None,
        };
        #[cfg(feature = "coverage")]
        {
            run.lease = run.enable_coverage();
        }
        run
    }

    /// The generator, for loops that shrink or mutate by hand.
    pub fn generator(&self) -> &G {
        &self.generator
    }

    /// The failing input kept by [`test`](Self::test), as found.
    pub fn failure(&self) -> Option<&G::Value> {
        self.pending.as_ref().map(|found| &found.input.value)
    }

    /// The report so far.
    pub fn report(&self) -> &Report {
        &self.report
    }

    /// Shrink, localize and remember the failure if there was one, then
    /// the final statistics, health checks and report output (JSON,
    /// observations, profile, the coverage grid).
    pub fn finish(mut self) -> Report {
        if self.pending.is_some() {
            self.fail();
        }
        if self.report.target.is_none() && self.mutations > 0 {
            self.report.target = self.target_summary(self.mutations);
        }
        self.summarize();
        #[cfg(feature = "coverage")]
        self.finish_coverage();
        health::assess(&mut self.report);
        #[cfg(feature = "json")]
        if let Some(path) = &self.config.report.json_path {
            use std::io::Write;
            let line = serde_json::to_string(&self.report).expect("Report is always serialisable");
            let written = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .and_then(|mut file| writeln!(file, "{line}"));
            if let Err(e) = written {
                self.report
                    .warn("report_json", format!("could not append to {}: {e}", path.display()));
            }
        }
        for mut sink in std::mem::take(&mut self.exec.sinks) {
            if let Err(e) = sink.finish(&self.report) {
                self.report.warn("observations", e);
            }
        }
        self.report
    }
}

/// Steps for loops over traces (see [`Trace`](crate::Trace)).
impl<G, R, P> Run<G, P>
where
    G: crate::Trace,
    G::Value: Clone + Debug,
    R: Testable,
    P: FnMut(G::Value) -> R,
{
    /// Rewind the failing input kept by [`test`](Self::test) to its first
    /// `prefix` steps, continue it with fresh randomness
    /// ([`Trace::resume`](crate::Trace::resume)) and run the property on
    /// the continuation: `Some(true)` when it fails the way the failure did
    /// (same kind, same panic site), `Some(false)` when it passes or fails
    /// otherwise. `None` without a failure, when the failure did not
    /// reproduce, or when the continuation was discarded or its generation
    /// panicked (which aborts the run). Continuations are not counted as
    /// tests; with coverage they feed fault localization like mutants do.
    pub fn branch(&mut self, prefix: usize) -> Option<bool> {
        if !self.reproduce_pending() {
            return None;
        }
        let signature = self.shrinking.as_ref()?.signature.clone();
        let (parent, at) = {
            let found = self.pending.as_ref()?;
            (found.input.value.clone(), found.derived("branch", self.report.seed))
        };
        let iteration = self.next_iteration();
        let origin = crate::report::Origin::Branch;
        let made = self.produce(self.report.seed, iteration, origin, |g, source| {
            g.resume(&parent, prefix, source)
        });
        let input = match made {
            Ok(input) => input,
            Err(Skipped::Discarded(reason, _)) => {
                self.record_discard(reason);
                return None;
            },
            Err(Skipped::Panicked(info)) => {
                self.abort(info, iteration);
                return None;
            },
        };
        let ran = self.exec.run(input.value, at, None);
        self.report.prop_time += ran.elapsed;
        self.stats.prop_micros.push(micros(ran.elapsed));
        Some(Signature::of(&ran.result).as_ref() == Some(&signature))
    }

    /// Keep a causality analysis for the report.
    pub(crate) fn record_causality(&mut self, causality: crate::report::Causality) {
        self.report.causality = Some(causality);
    }
}

#[cfg(test)]
mod tests;
