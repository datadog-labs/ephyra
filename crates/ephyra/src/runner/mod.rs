//! The test loop: [`check`] and [`quickcheck`].

mod check;
pub mod labels;
mod panic;
pub mod testable;

use std::{
    collections::{
        BTreeMap,
        HashSet,
    },
    fmt::{
        self,
        Debug,
        Write as _,
    },
    hash::{
        BuildHasher,
        DefaultHasher,
        Hasher,
        RandomState,
    },
    time::Instant,
};

use {
    rand_chacha::ChaCha8Rng,
    rand_core::SeedableRng,
};

use crate::{
    Arbitrary,
    Config,
    Gen,
    Source,
    any,
    config::ReplayPoint,
    health,
    pool::Pool,
    report::{
        Abort,
        Counterexample,
        Failure,
        FailureKind,
        Location,
        ReplayOutcome,
        Report,
        ShrinkStats,
        Status,
        TargetSummary,
    },
    shrink,
    source::u01,
};
pub use testable::{
    Outcome,
    TestResult,
    Testable,
};

#[doc(hidden)]
pub use self::check::{
    __check_generator,
    __check_strategy,
};
use self::panic::PanicInfo;

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

/// Per-run coverage state: the tracker, the fault-localization spectrum and
/// the signature of the failure being localized.
#[cfg(feature = "coverage")]
#[derive(Default)]
struct CoverageState {
    tracker: Option<crate::coverage::tracker::Tracker>,
    spectrum: Option<crate::coverage::locate::Spectrum>,
    targets: Option<crate::coverage::targets::Targets>,
    #[cfg(feature = "json")]
    grid: Option<crate::coverage::grid::Grid>,
    signature: Option<Signature>,
}

#[cfg(feature = "coverage")]
impl CoverageState {
    fn begin(&self) -> Option<crate::coverage::Snapshot> {
        self.tracker.as_ref().and_then(|t| t.begin())
    }

    /// Record what an input did: whether it reached new coverage and, when
    /// asked, the source lines it executed. Passing runs and runs failing
    /// like the first failure feed the spectrum; discards and other failures
    /// do not.
    fn end(
        &mut self,
        before: Option<crate::coverage::Snapshot>,
        result: &Result<Outcome, PanicInfo>,
        want_lines: bool,
    ) -> Covered {
        let (Some(tracker), Some(before)) = (self.tracker.as_mut(), before) else {
            return Covered::default();
        };
        let observation = tracker.end(&before);
        let target_hit =
            self.targets.as_mut().is_some_and(|t| t.observe(&observation.delta, tracker.inputs()));
        #[cfg(feature = "json")]
        if let Some(grid) = self.grid.as_mut() {
            grid.observe(&observation.delta, tracker.selected());
        }
        let lines = want_lines
            .then(|| crate::coverage::lines_hit(&observation.delta, tracker.selected()))
            .flatten();
        if let Some(spectrum) = self.spectrum.as_mut() {
            match Signature::of(result) {
                None => {
                    if matches!(result, Ok(Outcome::Pass)) {
                        spectrum.observe(&observation.delta, false);
                    }
                },
                Some(signature) => {
                    let same = match &self.signature {
                        Some(first) => *first == signature,
                        None => {
                            self.signature = Some(signature);
                            true
                        },
                    };
                    if same {
                        spectrum.observe(&observation.delta, true);
                    }
                },
            }
        }
        Covered { novelty: observation.novelty + u32::from(target_hit), lines }
    }
}

/// What [`CoverageState::end`] learned about one input.
#[derive(Default)]
struct Covered {
    /// Counters, hit-count classes and targets reached for the first time.
    novelty: u32,
    lines: Option<BTreeMap<String, Vec<u32>>>,
}

#[cfg(not(feature = "coverage"))]
#[derive(Default)]
struct CoverageState {}

#[cfg(not(feature = "coverage"))]
impl CoverageState {
    fn begin(&self) -> Option<()> {
        None
    }

    fn end(
        &mut self,
        _before: Option<()>,
        _result: &Result<Outcome, PanicInfo>,
        _want_lines: bool,
    ) -> Covered {
        Covered::default()
    }
}

/// The crate a type was defined in, read off its `type_name`: the first
/// path segment after any reference or pointer sigils.
#[cfg(feature = "coverage")]
pub(crate) fn crate_of(type_name: &str) -> &str {
    let bare = type_name
        .trim_start_matches(['&', '*', '<'])
        .trim_start_matches("mut ")
        .trim_start_matches("dyn ");
    let end = bare.find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(bare.len());
    &bare[..end]
}

#[cfg(all(test, feature = "coverage"))]
fn crate_of_probe_name() -> &'static str {
    let probe = |x: i32| x;
    let _ = probe(0);
    std::any::type_name_of_val(&probe)
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

/// The property, with what is collected around each run of it: coverage
/// and the sinks.
struct Exec<P> {
    property: P,
    coverage: CoverageState,
    /// Where each evaluated input goes: the observation file, when
    /// `Config::report.observations` is set, and the installed sinks.
    sinks: Vec<Box<dyn crate::sink::RunSink>>,
}

/// One run of the property.
struct Ran {
    result: Result<Outcome, PanicInfo>,
    observed: labels::Observed,
    elapsed: std::time::Duration,
    /// See [`Tested::novelty`].
    novelty: u32,
}

impl<P> Exec<P> {
    /// Whether any sink is open, so inputs are rendered and observed.
    fn observing(&self) -> bool {
        !self.sinks.is_empty()
    }

    /// Run the property on `value` with panic capture and coverage, and pass
    /// it to the sinks. `shrinking` is the failure a shrink candidate is
    /// compared with.
    fn run<T: Debug, R: Testable>(&mut self, value: T, at: At, shrinking: Option<&Signature>) -> Ran
    where
        P: FnMut(T) -> R,
    {
        let observing = self.observing();
        let representation = if observing { format!("{value:?}") } else { String::new() };
        let before = self.coverage.begin();
        let started = Instant::now();
        let (result, observed) = evaluate(&mut self.property, value, observing);
        let elapsed = started.elapsed();
        let covered = self.coverage.end(before, &result, observing);
        if observing {
            let (status, reason) = case_status(&result);
            self.emit(&crate::sink::TestCase {
                status,
                status_reason: &reason,
                representation: &representation,
                origin: at.origin,
                labels: &observed.labels,
                features: &observed.features,
                covers: &observed.covers,
                score: observed.score,
                observations: &observed.observations,
                coverage: covered.lines.as_ref(),
                generate: at.generate,
                execute: elapsed,
                seed: at.seed,
                iteration: at.iteration,
                size: at.size,
                shrink_accepted: shrinking.map(|s| Signature::of(&result).as_ref() == Some(s)),
            });
        }
        Ran { result, observed, elapsed, novelty: covered.novelty }
    }

    fn emit(&mut self, case: &crate::sink::TestCase<'_>) {
        for sink in &mut self.sinks {
            sink.test_case(case);
        }
    }
}

/// An input's status and reason, as sinks receive them.
fn case_status(result: &Result<Outcome, PanicInfo>) -> (crate::sink::CaseStatus, String) {
    use crate::sink::CaseStatus;
    match result {
        Ok(Outcome::Pass) => (CaseStatus::Passed, String::new()),
        Ok(Outcome::Discard(reason)) => (CaseStatus::GaveUp, reason.clone()),
        Ok(Outcome::False) => (CaseStatus::Failed, "the property returned false".to_string()),
        Ok(Outcome::Fail(message)) => (CaseStatus::Failed, message.clone()),
        Err(info) => {
            (
                CaseStatus::Failed,
                match &info.location {
                    Some(at) => format!("panicked at {at}: {}", info.message),
                    None => format!("panicked: {}", info.message),
                },
            )
        },
    }
}

/// Where an evaluated input came from, as sinks receive it.
#[derive(Clone, Copy)]
struct At {
    origin: &'static str,
    seed: u64,
    iteration: u64,
    size: usize,
    /// How long generating it took.
    generate: std::time::Duration,
}

/// Where a sampled or slow input can be rendered from later.
#[derive(Clone)]
enum Provenance {
    /// Regenerate from the point (random-phase inputs).
    Point(ReplayPoint),
    /// Already rendered (mutants).
    Debug(String),
}

#[derive(Clone)]
struct Candidate {
    origin: crate::report::Origin,
    size: usize,
    iteration: u64,
    source: Provenance,
}

/// The inputs shown as samples: the smallest and largest by size plus a
/// reservoir sample of the rest.
#[derive(Default)]
struct Samples {
    limit: usize,
    smallest: Option<Candidate>,
    largest: Option<Candidate>,
    reservoir: Vec<Candidate>,
    seen: u64,
}

impl Samples {
    fn new(limit: usize) -> Samples {
        Samples { limit, ..Samples::default() }
    }

    /// Offer a passing input; `source` is only called when it is kept.
    fn consider(
        &mut self,
        size: usize,
        iteration: u64,
        origin: crate::report::Origin,
        r: f64,
        source: impl FnOnce() -> Provenance,
    ) {
        if self.limit == 0 {
            return;
        }
        self.seen += 1;
        let smaller = self.smallest.as_ref().is_none_or(|c| size < c.size);
        let larger = self.limit >= 2 && self.largest.as_ref().is_none_or(|c| size > c.size);
        let k = self.limit.saturating_sub(2);
        let slot = if k == 0 {
            None
        } else if self.reservoir.len() < k {
            Some(self.reservoir.len())
        } else {
            let j = (r * self.seen as f64) as usize;
            (j < k).then_some(j)
        };
        if !(smaller || larger || slot.is_some()) {
            return;
        }
        let candidate = Candidate { origin, size, iteration, source: source() };
        if smaller {
            self.smallest = Some(candidate.clone());
        }
        if larger {
            self.largest = Some(candidate.clone());
        }
        match slot {
            Some(j) if j < self.reservoir.len() => self.reservoir[j] = candidate,
            Some(_) => self.reservoir.push(candidate),
            None => {},
        }
    }

    /// Everything kept, smallest first, without repeats.
    fn drain(&mut self) -> Vec<Candidate> {
        let mut out: Vec<Candidate> = Vec::new();
        for candidate in self
            .smallest
            .take()
            .into_iter()
            .chain(self.largest.take())
            .chain(self.reservoir.drain(..))
        {
            if !out
                .iter()
                .any(|c| c.origin == candidate.origin && c.iteration == candidate.iteration)
            {
                out.push(candidate);
            }
        }
        out
    }
}

/// Per-run accumulators behind the report's distributions.
#[derive(Default)]
struct Stats {
    /// Debug hashes of passing inputs.
    passed_seen: HashSet<u64>,
    /// Numeric feature values over passing tests.
    features: BTreeMap<String, Vec<f64>>,
    gen_micros: Vec<u32>,
    prop_micros: Vec<u32>,
    slowest: Option<(u32, Provenance)>,
    distinct_growth: crate::report::Series,
    samples: Samples,
}

impl Stats {
    fn new(samples: usize) -> Stats {
        Stats { samples: Samples::new(samples), ..Stats::default() }
    }

    /// Note an input's property time; `source` is only called for a new
    /// maximum.
    fn slower(&mut self, elapsed: std::time::Duration, source: impl FnOnce() -> Provenance) {
        let micros = micros(elapsed);
        if self.slowest.as_ref().is_none_or(|(best, _)| micros > *best) {
            self.slowest = Some((micros, source()));
        }
    }
}

/// Samples are for reading, so they are cut shorter than counterexamples.
const SAMPLE_CHARS: usize = 200;

/// Cut a rendering to `limit` characters, marking the cut.
fn cap(mut debug: String, limit: usize) -> String {
    if let Some((cut, _)) = debug.char_indices().nth(limit) {
        debug.truncate(cut);
        debug.push('…');
    }
    debug
}

fn micros(d: std::time::Duration) -> u32 {
    d.as_micros().min(u128::from(u32::MAX)) as u32
}

/// A uniform draw in `[0, 1)` from a seed and an index.
fn hash_u01(seed: u64, index: u64) -> f64 {
    (mix(seed, index) >> 11) as f64 / (1u64 << 53) as f64
}

/// A failing input, kept for [`Run::finish`].
struct Found<T> {
    input: Input<T>,
    result: Result<Outcome, PanicInfo>,
}

impl<T> Found<T> {
    /// Where inputs derived from the failure (shrink candidates, neighbours,
    /// the minimal input) sit in the observation file.
    fn derived(&self, origin: &'static str, seed: u64) -> At {
        let (iteration, size) = (self.input.iteration, self.input.size);
        At { origin, seed, iteration, size, generate: std::time::Duration::ZERO }
    }
}

/// The pending failure's shrinking: what it fails like, the smallest input
/// known to fail the same way, and the steps taken so far.
struct Shrinking<T> {
    signature: Signature,
    /// The failure did not reproduce when rerun; nothing is shrunk.
    flaky: bool,
    minimal: T,
    /// The message and location of the failure `minimal` produced, once a
    /// candidate was accepted.
    failure: shrink::SameFailure,
    stats: ShrinkStats,
    /// The built-in driver ran, or the loop supplied its own result.
    done: bool,
    started: Option<Instant>,
}

/// Why an input could not be generated.
enum Skipped {
    /// The generator discarded it, after the given time.
    Discarded(String, std::time::Duration),
    Panicked(PanicInfo),
}

/// An input drawn or mutated by a [`Run`], ready to be tested.
#[derive(Clone, Debug)]
pub struct Input<T> {
    /// The value.
    pub value: T,
    point: Option<ReplayPoint>,
    size: usize,
    iteration: u64,
    origin: crate::report::Origin,
    generate: std::time::Duration,
}

impl<T> Input<T> {
    /// The size the generator was asked for.
    pub fn size(&self) -> usize {
        self.size
    }

    /// The run's iteration that produced it.
    pub fn iteration(&self) -> u64 {
        self.iteration
    }

    /// Where it regenerates from, for inputs from [`Run::draw`]; mutants
    /// have none.
    pub fn replay(&self) -> Option<ReplayPoint> {
        self.point
    }

    /// How it came to be.
    pub fn origin(&self) -> crate::report::Origin {
        self.origin
    }

    /// Where it sits in the observation file, recorded as `origin`; mutants
    /// carry the run's `seed`.
    fn at(&self, origin: &'static str, seed: u64) -> At {
        let seed = self.point.map_or(seed, |p| p.seed);
        At { origin, seed, iteration: self.iteration, size: self.size, generate: self.generate }
    }
}

/// The three ways testing one input can end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Verdict {
    /// The property held.
    Passed,
    /// The property discarded the input.
    Discarded,
    /// The property failed; the run keeps the first such input for
    /// [`Run::finish`].
    Failed,
}

/// What one test of an input showed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tested {
    verdict: Verdict,
    novelty: u32,
    score: Option<f64>,
    elapsed: std::time::Duration,
}

impl Tested {
    /// Whether the property held.
    pub fn passed(&self) -> bool {
        self.verdict == Verdict::Passed
    }

    /// Whether the property discarded the input.
    pub fn discarded(&self) -> bool {
        self.verdict == Verdict::Discarded
    }

    /// Whether the property failed.
    pub fn failed(&self) -> bool {
        self.verdict == Verdict::Failed
    }

    /// Whether the input executed code no earlier input had (coverage on,
    /// instrumented build); otherwise always false.
    pub fn novel(&self) -> bool {
        self.novelty > 0
    }

    /// How many counters, hit-count classes of a counter, or coverage
    /// targets the input reached for the first time in this run: the size
    /// of its novelty. Zero without coverage.
    pub fn novelty(&self) -> u32 {
        self.novelty
    }

    /// How long the property took on the input.
    pub fn elapsed(&self) -> std::time::Duration {
        self.elapsed
    }

    /// The score the property reported through [`target`](crate::target),
    /// for passing inputs.
    pub fn score(&self) -> Option<f64> {
        self.score
    }
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

    /// Run `make` (the generator's `generate` or `mutate`) for `iteration`
    /// with randomness from `seed`, counting its time as generation.
    fn produce(
        &mut self,
        seed: u64,
        iteration: u64,
        origin: crate::report::Origin,
        make: impl FnOnce(&G, &mut Source) -> G::Value,
    ) -> Result<Input<G::Value>, Skipped> {
        let size = size_for(iteration, self.config.max_size);
        let mut source = Source::new(mix(seed, iteration), size)
            .with_filter_max_attempts(self.config.filter_max_attempts);
        let started = Instant::now();
        let made = panic::catch(|| make(&self.generator, &mut source));
        let generate = started.elapsed();
        self.report.gen_time += generate;
        self.stats.gen_micros.push(micros(generate));
        match (made, source.discarded()) {
            (Err(info), _) => Err(Skipped::Panicked(info)),
            (Ok(_), Some(reason)) => Err(Skipped::Discarded(reason.to_string(), generate)),
            (Ok(value), None) => {
                Ok(Input { value, point: None, size, iteration, origin, generate })
            },
        }
    }

    /// Generate the input at `point`.
    fn generate(
        &mut self,
        point: ReplayPoint,
        origin: crate::report::Origin,
    ) -> Result<Input<G::Value>, Skipped> {
        let input =
            self.produce(point.seed, point.iteration, origin, |g, source| g.generate(source))?;
        Ok(Input { point: Some(point), ..input })
    }

    /// The value at `point`, if it generates without discard or panic.
    fn regenerate(&mut self, point: ReplayPoint) -> Option<G::Value> {
        self.generate(point, crate::report::Origin::Replay).ok().map(|input| input.value)
    }

    /// Run the property on `value` (see [`Exec::run`]), counting its time.
    fn run_property(&mut self, value: G::Value, at: At) -> Ran {
        let ran = self.exec.run(value, at, None);
        self.report.prop_time += ran.elapsed;
        self.stats.prop_micros.push(micros(ran.elapsed));
        ran
    }

    /// Count a generated input, and the growth of the distinct set.
    fn note_distinct(&mut self, hash: u64) {
        if self.seen.insert(hash) {
            self.stats.distinct_growth.push(self.generated, self.seen.len() as u64);
        }
    }

    fn next_iteration(&mut self) -> u64 {
        let iteration = self.report.iterations;
        self.report.iterations += 1;
        iteration
    }

    /// The observation line of an input the generator discarded after
    /// `generate`.
    fn observe_generation_discard(
        &mut self,
        point: ReplayPoint,
        origin: crate::report::Origin,
        reason: &str,
        generate: std::time::Duration,
    ) {
        if self.exec.observing() {
            self.exec.emit(&crate::sink::TestCase {
                status: crate::sink::CaseStatus::GaveUp,
                status_reason: reason,
                representation: "",
                origin: origin.as_str(),
                labels: &[],
                features: &[],
                covers: &[],
                score: None,
                observations: &[],
                coverage: None,
                generate,
                execute: std::time::Duration::ZERO,
                seed: point.seed,
                iteration: point.iteration,
                size: size_for(point.iteration, self.config.max_size),
                shrink_accepted: None,
            });
        }
    }

    /// The generator panicked at `iteration`: the run aborts.
    fn abort(&mut self, info: PanicInfo, iteration: u64) {
        self.report.status = Status::Aborted;
        self.report.abort =
            Some(Abort { message: info.message, location: info.location, iteration });
    }

    /// Draw the input at `point`. A discard by the generator is counted
    /// and a generator panic aborts the run; both yield `None`.
    fn draw_at(
        &mut self,
        point: ReplayPoint,
        origin: crate::report::Origin,
    ) -> Option<Input<G::Value>> {
        match self.generate(point, origin) {
            Ok(input) => Some(input),
            Err(Skipped::Discarded(reason, generate)) => {
                self.observe_generation_discard(point, origin, &reason, generate);
                self.record_discard(reason);
                None
            },
            Err(Skipped::Panicked(info)) => {
                self.abort(info, point.iteration);
                None
            },
        }
    }

    /// Draw the next random input, at this iteration's size.
    /// `None` when the generator discarded it (counted toward the discard
    /// limit) or panicked (the run aborts; see [`wants_more`](Self::wants_more)).
    pub fn draw(&mut self) -> Option<Input<G::Value>> {
        let iteration = self.next_iteration();
        self.draw_at(
            ReplayPoint { seed: self.report.seed, iteration },
            crate::report::Origin::Random,
        )
    }

    /// Run the property on `input`. A pass counts toward the run's tests
    /// and records the input's labels, features, timing and coverage; a
    /// discard counts toward the discard limit; the first failure is kept
    /// for [`finish`](Self::finish) to shrink and report, and later ones
    /// are only counted.
    pub fn test(&mut self, input: &Input<G::Value>) -> Tested {
        let (tested, result) = self.test_input(input, true);
        if tested.failed() && self.pending.is_none() {
            self.pending = Some(Found { input: input.clone(), result });
        }
        tested
    }

    /// Test `input`, counting passes and discards when `record`. Returns
    /// the raw result alongside the verdict for callers that keep failures
    /// themselves.
    fn test_input(
        &mut self,
        input: &Input<G::Value>,
        record: bool,
    ) -> (Tested, Result<Outcome, PanicInfo>) {
        self.generated += 1;
        let hash = debug_hash(&input.value);
        self.note_distinct(hash);
        let at = input.at(input.origin.as_str(), self.report.seed);
        let Ran { result, observed, elapsed, novelty } = self.run_property(input.value.clone(), at);
        let limit = self.config.report.truncate_debug_at;
        let (value, point) = (&input.value, input.point);
        let provenance = || {
            match point {
                Some(point) => Provenance::Point(point),
                None => Provenance::Debug(cap(format!("{value:?}"), limit)),
            }
        };
        self.stats.slower(elapsed, provenance);
        let score = observed.score;
        match result {
            Ok(Outcome::Pass) => {
                if record {
                    self.record_pass(
                        input.size,
                        hash,
                        observed,
                        input.origin,
                        input.iteration,
                        provenance,
                    );
                }
                (Tested { verdict: Verdict::Passed, novelty, score, elapsed }, Ok(Outcome::Pass))
            },
            Ok(Outcome::Discard(reason)) => {
                if record {
                    self.record_discard(reason.clone());
                }
                (
                    Tested { verdict: Verdict::Discarded, novelty, score: None, elapsed },
                    Ok(Outcome::Discard(reason)),
                )
            },
            result => (Tested { verdict: Verdict::Failed, novelty, score: None, elapsed }, result),
        }
    }

    fn record_discard(&mut self, reason: String) {
        self.report.discarded += 1;
        *self.report.discard_reasons.entry(reason).or_default() += 1;
    }

    fn record_pass(
        &mut self,
        size: usize,
        hash: u64,
        observed: labels::Observed,
        origin: crate::report::Origin,
        iteration: u64,
        source: impl FnOnce() -> Provenance,
    ) {
        self.report.tests += 1;
        self.stats.passed_seen.insert(hash);
        *self.report.sizes.entry(size).or_default() += 1;
        for label in observed.labels {
            *self.report.labels.entry(label).or_default() += 1;
        }
        for (label, met) in observed.covers {
            let entry = self.covers.entry(label).or_insert((0, 0));
            entry.0 += u64::from(met);
            entry.1 += 1;
        }
        for (name, value) in observed.features {
            match value {
                labels::FeatureValue::Number(n) => {
                    self.stats.features.entry(name).or_default().push(n);
                },
                labels::FeatureValue::Text(text) => {
                    *self.report.labels.entry(format!("{name}={text}")).or_default() += 1;
                },
            }
        }
        if let Some(score) = observed.score {
            self.saw_score = true;
            self.stats.features.entry("target".to_string()).or_default().push(score);
        }
        let r = hash_u01(self.report.seed ^ 0x5a5a_5a5a, self.report.tests);
        self.stats.samples.consider(size, iteration, origin, r, source);
    }

    /// Whether coverage-guided search is on and coverage is being collected.
    fn guided(&self) -> bool {
        #[cfg(feature = "coverage")]
        {
            (self.config.coverage.guided || self.exec.coverage.targets.is_some())
                && self.exec.coverage.tracker.is_some()
        }
        #[cfg(not(feature = "coverage"))]
        {
            false
        }
    }

    #[cfg(feature = "coverage")]
    /// Start collecting coverage if configured and available. Returns the
    /// process-wide coverage lock, which the caller holds for the run.
    fn enable_coverage(&mut self) -> Option<crate::coverage::Lease> {
        let settings = &self.config.coverage;
        if !settings.enabled {
            return None;
        }
        if !crate::coverage::available() {
            if settings.guided
                || settings.profraw_dir.is_some()
                || !settings.targets.is_empty()
                || settings.html.is_some()
            {
                self.report.warn(
                    "coverage_unavailable",
                    "coverage was requested but this binary is not instrumented; build with RUSTFLAGS=\"-C instrument-coverage\"",
                );
            }
            return None;
        }
        let property_type = std::any::type_name::<P>();
        let filter = if settings.filter.is_empty() {
            crate_of(property_type).to_string()
        } else {
            settings.filter.clone()
        };
        let targets = self.resolve_targets();
        let mut extra = targets.as_ref().map(|t| t.functions()).unwrap_or_default();
        // The property's own function is tracked whichever crate it lives
        // in, so the grid can point at it; it stays out of fault
        // localization, where it could only be noise.
        let property_fns: Vec<usize> = crate::coverage::counters::Functions::get()
            .all()
            .iter()
            .enumerate()
            .filter(|(_, f)| crate::coverage::is_property_fn(&f.name, property_type))
            .map(|(i, _)| i)
            .collect();
        extra.extend(property_fns.iter().copied());
        match crate::coverage::tracker::Tracker::new(&filter, &extra) {
            Some(tracker) => {
                let guard = crate::coverage::lock();
                self.exec.coverage.targets = targets;
                if self.config.coverage.html.is_some() {
                    #[cfg(feature = "json")]
                    {
                        self.exec.coverage.grid = crate::coverage::grid::Grid::new(&property_fns);
                        if self.exec.coverage.grid.is_none() {
                            self.report.warn(
                                "coverage_html",
                                "the binary carries no coverage mapping, so the coverage grid cannot be written",
                            );
                        }
                    }
                    #[cfg(not(feature = "json"))]
                    self.report.warn("coverage_html", "the coverage grid needs the `json` feature");
                }
                if self.config.locate.enabled {
                    let code_under_test: Vec<usize> = tracker
                        .selected()
                        .iter()
                        .copied()
                        .filter(|i| !property_fns.contains(i))
                        .collect();
                    self.exec.coverage.spectrum =
                        crate::coverage::locate::Spectrum::new(&code_under_test);
                    if self.exec.coverage.spectrum.is_none() {
                        self.report.warn(
                            "locate_unavailable",
                            "the binary carries no coverage mapping, so failures cannot be localized",
                        );
                    }
                }
                self.exec.coverage.tracker = Some(tracker);
                // The test harness runs each test on a thread named after it;
                // a plain binary runs on `main` and has no sibling tests.
                let in_test_harness = std::thread::current().name().is_some_and(|n| n != "main");
                if in_test_harness
                    && std::env::var("RUST_TEST_THREADS").ok().as_deref() != Some("1")
                {
                    self.report.warn(
                        "coverage_threads",
                        "coverage is attributed per input, and other test threads running at the same time pollute it; \
                         run with RUST_TEST_THREADS=1 for exact attribution",
                    );
                }
                Some(guard)
            },
            None => {
                self.report.warn(
                    "coverage_filter",
                    format!("no instrumented functions match the coverage filter `{filter}`"),
                );
                None
            },
        }
    }

    /// The configured coverage targets (inline and from the file), resolved.
    #[cfg(feature = "coverage")]
    fn resolve_targets(&mut self) -> Option<crate::coverage::targets::Targets> {
        #[cfg_attr(not(feature = "json"), allow(unused_mut))]
        let mut targets = self.config.coverage.targets.clone();
        if let Some(path) = self.config.coverage.targets_file.clone() {
            #[cfg(feature = "json")]
            match crate::config::load_targets(&path) {
                Ok(more) => targets.extend(more),
                Err(e) => {
                    self.report
                        .warn("coverage_targets", format!("could not read the targets file: {e}"))
                },
            }
            #[cfg(not(feature = "json"))]
            if path.exists() {
                self.report.warn(
                    "coverage_targets",
                    format!("reading {} needs the `json` feature", path.display()),
                );
            }
        }
        if targets.is_empty() {
            return None;
        }
        match crate::coverage::targets::Targets::resolve(targets) {
            Some(resolved) => {
                for target in resolved.unresolved() {
                    self.report.warn(
                        "coverage_target",
                        format!(
                            "target {target} matches no instrumented code; check the path and lines"
                        ),
                    );
                }
                Some(resolved)
            },
            None => {
                self.report.warn(
                    "coverage_targets_unavailable",
                    "the binary carries no coverage mapping, so coverage targets cannot be resolved",
                );
                None
            },
        }
    }

    /// Whether the run still wants inputs: none of the stopping rules has
    /// fired. The rules, in order: the number of tests reached
    /// [`Config::tests`]; the time budget ([`Config::budget_ms`]) ran out;
    /// the timeout ([`Config::timeout_ms`]) elapsed; more inputs were
    /// discarded than [`Config::max_discard_ratio`] allows; the property
    /// failed, or the generator panicked. A rule that fired is reflected in
    /// the report.
    pub fn wants_more(&mut self) -> bool {
        if self.pending.is_some() || self.report.status != Status::Passed {
            return false;
        }
        if self.report.tests >= self.config.tests {
            return false;
        }
        if let Some(budget) = self.config.budget_ms.map(std::time::Duration::from_millis)
            && self.started.elapsed() > budget
        {
            self.report.stopped_by_budget = true;
            return false;
        }
        !self.timed_out_or_gave_up(self.report.discarded, self.config.tests)
    }

    /// The timeout and discard-limit stopping rules, shared by the random and
    /// targeted phases: whether to stop, with the reason set as the status.
    fn timed_out_or_gave_up(&mut self, discarded: u64, tests: u64) -> bool {
        if let Some(timeout) = self.config.timeout_duration()
            && self.started.elapsed() > timeout
        {
            self.report.status = Status::TimedOut;
            return true;
        }
        let max_discards = (self.config.max_discard_ratio * tests as f64).max(0.0) as u64;
        if discarded > max_discards {
            self.report.status = Status::GaveUp;
            return true;
        }
        false
    }

    /// The targeted phase's summary after `iterations`, once the pool has a best.
    fn target_summary(&self, iterations: u64) -> Option<TargetSummary> {
        let (best_score, best) = self.pool.best.as_ref()?;
        Some(TargetSummary {
            scored: self.saw_score,
            iterations,
            improvements: self.pool.improvements,
            best_score: *best_score,
            best_debug: cap(format!("{best:?}"), self.config.report.truncate_debug_at),
        })
    }

    /// The random phase: draw, test, keep, until a stopping rule fires.
    fn random_search(&mut self) {
        while self.wants_more() {
            let iteration = self.next_iteration();
            let point = ReplayPoint { seed: self.report.seed, iteration };
            let Some(input) = self.draw_at(point, crate::report::Origin::Random) else {
                continue;
            };
            let tested = self.test(&input);
            if tested.passed() {
                self.keep(input.value, &tested);
            }
        }
    }

    /// Keep a passing input as a seed for mutation when its score improves
    /// the pool, or when guided search is on and it reached new coverage.
    /// Returns whether it was kept.
    pub fn keep(&mut self, value: G::Value, tested: &Tested) -> bool {
        if !tested.passed() {
            return false;
        }
        let guided = self.guided();
        let by_score = tested.score.is_some_and(|s| self.pool.improves(s));
        if by_score {
            self.pool.add(value, tested.score.unwrap_or(0.0));
            true
        } else if tested.novel() && guided {
            self.pool.add_forced(value, tested.score.unwrap_or(0.0));
            true
        } else {
            false
        }
    }

    /// Keep an input as a seed with `score`, whatever the pool's threshold.
    pub fn keep_always(&mut self, value: G::Value, score: f64) {
        self.pool.add_forced(value, score);
    }

    /// Pick a seed to mutate, weighted by score; each seed has a budget of
    /// picks. `None` when the pool is empty or every budget is spent.
    pub fn pool_pick(&mut self) -> Option<G::Value> {
        self.pool.pick(&mut self.rng).map(|seed| seed.value)
    }

    /// A variant of `parent` from the generator's mutator, at this
    /// iteration's size. The mutator itself returns a value;
    /// this runner wrapper returns `None` if it discarded or panicked
    /// (a panic aborts the run).
    pub fn mutate(&mut self, parent: &G::Value) -> Option<Input<G::Value>> {
        let iteration = self.next_iteration();
        self.mutations += 1;
        self.mutate_at(parent, iteration)
    }

    fn mutate_at(&mut self, parent: &G::Value, iteration: u64) -> Option<Input<G::Value>> {
        let origin = crate::report::Origin::Mutation;
        match self
            .produce(self.report.seed, iteration, origin, |g, source| g.mutate(parent, source))
        {
            Ok(input) => Some(input),
            Err(Skipped::Discarded(reason, _)) => {
                self.record_discard(reason);
                None
            },
            Err(Skipped::Panicked(info)) => {
                self.abort(info, iteration);
                None
            },
        }
    }

    /// The generator, for loops that shrink or mutate by hand.
    pub fn generator(&self) -> &G {
        &self.generator
    }

    /// The failing input kept by [`test`](Self::test), as found.
    pub fn failure(&self) -> Option<&G::Value> {
        self.pending.as_ref().map(|found| &found.input.value)
    }

    /// Rerun the pending failure once to see that it reproduces (the
    /// flakiness check) and set up shrinking. Returns whether there is a
    /// failure to shrink.
    fn reproduce_pending(&mut self) -> bool {
        if let Some(shrinking) = &self.shrinking {
            return !shrinking.flaky;
        }
        let Some(found) = self.pending.as_ref() else {
            return false;
        };
        let signature = Signature::of(&found.result).expect("pending failures fail");
        let value = found.input.value.clone();
        let started = Instant::now();
        let (rerun, _) = evaluate(&mut self.exec.property, value.clone(), false);
        self.report.prop_time += started.elapsed();
        self.stats.prop_micros.push(micros(started.elapsed()));
        let flaky = Signature::of(&rerun).as_ref() != Some(&signature);
        if flaky {
            self.report.warn(
                "flaky",
                "the property did not fail the same way when re-run on the failing input; \
                 it depends on something other than the input, and the input was not shrunk",
            );
        }
        self.shrinking = Some(Shrinking {
            signature,
            flaky,
            minimal: value,
            failure: None,
            stats: ShrinkStats::default(),
            done: false,
            started: None,
        });
        !flaky
    }

    /// Whether `candidate` fails the way the pending failure did (same
    /// kind of failure, same panic site), for loops that shrink by hand.
    /// Each call is a shrinking step: beyond [`Config::shrink`]'s budget
    /// the answer is always `false` and the report says the budget ran
    /// out. `false` without a failure, or when it did not reproduce.
    pub fn reproduces(&mut self, candidate: &G::Value) -> bool {
        if !self.reproduce_pending() {
            return false;
        }
        let Some(mut shrinking) = self.shrinking.take() else {
            return false;
        };
        let started = *shrinking.started.get_or_insert_with(Instant::now);
        let accepted = if self.config.shrink.exhausted(shrinking.stats.steps, started) {
            shrinking.stats.budget_exhausted = true;
            false
        } else {
            self.shrink_step(&mut shrinking, candidate.clone()).is_some()
        };
        self.shrinking = Some(shrinking);
        accepted
    }

    /// Test one shrink candidate of the pending failure, counting the step in
    /// `shrinking` and its time in the report.
    fn shrink_step(
        &mut self,
        shrinking: &mut Shrinking<G::Value>,
        candidate: G::Value,
    ) -> shrink::SameFailure {
        let at = self.pending.as_ref()?.derived("shrink", self.report.seed);
        let started = *shrinking.started.get_or_insert_with(Instant::now);
        shrinking.stats.steps += 1;
        let ran = self.exec.run(candidate, at, Some(&shrinking.signature));
        self.report.prop_time += ran.elapsed;
        let same = same_failure(ran.result, &shrinking.signature);
        if same.is_some() {
            shrinking.stats.accepted += 1;
        }
        shrinking.stats.duration = started.elapsed();
        same
    }

    /// Record `minimal` as the result of the loop's own shrinking. It is
    /// tested once more: when it fails the way the original did, the report
    /// shows it as the shrunk counterexample and [`finish`](Self::finish)
    /// shrinks no further; otherwise the original stands, with a warning.
    /// Returns whether it was accepted.
    pub fn shrunk_to(&mut self, minimal: G::Value) -> bool {
        if !self.reproduce_pending() {
            return false;
        }
        let Some(mut shrinking) = self.shrinking.take() else {
            return false;
        };
        let same = self.shrink_step(&mut shrinking, minimal.clone());
        shrinking.done = true;
        let accepted = same.is_some();
        match same {
            Some(failure) => {
                shrinking.minimal = minimal;
                shrinking.failure = Some(failure);
            },
            None => {
                self.report.warn(
                    "shrunk_to",
                    "the input given to shrunk_to does not fail the way the original did; \
                     the original is reported",
                );
            },
        }
        self.shrinking = Some(shrinking);
        accepted
    }

    /// Shrink the pending failure now with the built-in driver: greedy over
    /// the generator's candidates, within [`Config::shrink`]'s budget.
    /// Returns the smallest input that fails the same way; `None` without
    /// a failure, or when it did not reproduce. [`finish`](Self::finish)
    /// then reports this result and shrinks no further.
    pub fn shrink(&mut self) -> Option<&G::Value> {
        if !self.reproduce_pending() {
            return None;
        }
        let at = self.pending.as_ref()?.derived("shrink", self.report.seed);
        let mut shrinking = self.shrinking.take()?;
        if !shrinking.done && self.config.shrink.enabled {
            self.drive_shrink(&mut shrinking, at);
        }
        self.shrinking = Some(shrinking);
        self.shrinking.as_ref().map(|s| &s.minimal)
    }

    /// The built-in shrinking driver over `shrinking.minimal`; candidates are
    /// observed `at` the failure.
    fn drive_shrink(&mut self, shrinking: &mut Shrinking<G::Value>, at: At) {
        let generator = &self.generator;
        let exec = &mut self.exec;
        let signature = &shrinking.signature;
        let started = Instant::now();
        let result = shrink::shrink(
            &self.config.shrink,
            shrinking.minimal.clone(),
            |value, visitor| generator.shrink(value, visitor),
            |candidate| same_failure(exec.run(candidate, at, Some(signature)).result, signature),
        );
        self.report.prop_time += started.elapsed();
        shrinking.minimal = result.value;
        shrinking.failure = result.failure;
        shrinking.stats = result.stats;
        shrinking.done = true;
        if shrinking.stats.budget_exhausted {
            self.report.warn(
                "shrink_budget",
                format!(
                    "shrinking stopped at the budget ({} candidates); raise Config::shrink.max_steps or the \
                     timeout for a smaller counterexample",
                    shrinking.stats.steps
                ),
            );
        }
    }

    /// The fault-localization ranking as it stands: the functions and
    /// regions whose coverage separates the failing inputs tested so far
    /// from the passing ones, with the failure's panic site as a prior.
    /// `None` without a failure, or without coverage (`coverage` feature,
    /// instrumented build, [`Config::locate`] on).
    #[cfg(feature = "coverage")]
    pub fn suspects(&self) -> Option<crate::coverage::Locate> {
        let spectrum = self.exec.coverage.spectrum.as_ref()?;
        let found = self.pending.as_ref()?;
        let signature = Signature::of(&found.result)?;
        let panic = signature.panic_site.as_ref().map(|(file, line)| (file.as_str(), *line));
        Some(spectrum.rank(panic, self.config.locate.top))
    }

    /// The report so far.
    pub fn report(&self) -> &Report {
        &self.report
    }

    /// Test the remembered inputs first: the replay point
    /// ([`Config::replay`], `EPHYRA_REPLAY`) and the property's stored
    /// regressions. Neither counts as a test. Returns whether one failed,
    /// in which case the loop should stop and [`finish`](Self::finish)
    /// reports it.
    pub fn remembered(&mut self) -> bool {
        if let Some(point) = self.config.replay {
            let origin = crate::report::Origin::Replay;
            let failed = match self.generate(point, origin) {
                Ok(input) => {
                    let (tested, result) = self.test_input(&input, false);
                    tested.failed().then_some(Found { input, result })
                },
                Err(Skipped::Discarded(reason, generate)) => {
                    self.observe_generation_discard(point, origin, &reason, generate);
                    None
                },
                Err(Skipped::Panicked(_)) => None,
            };
            self.report.replay = Some(ReplayOutcome { point, reproduced: failed.is_some() });
            if failed.is_some() {
                self.pending = failed;
                return true;
            }
        }

        #[cfg(feature = "json")]
        if self.config.regressions.replay {
            let entries = crate::regressions::load(&self.config.regressions.dir, &self.report.name);
            if !entries.is_empty() {
                self.report.regressions.file = Some(crate::regressions::path_for(
                    &self.config.regressions.dir,
                    &self.report.name,
                ));
            }
            let limit = self.config.report.truncate_debug_at;
            for entry in entries {
                let point = ReplayPoint { seed: entry.seed, iteration: entry.iteration };
                self.report.regressions.replayed += 1;
                let Ok(input) = self.generate(point, crate::report::Origin::Replay) else {
                    self.report.regressions.stale += 1;
                    self.report.warn("regressions", format!("stored input {point} could not be regenerated (discarded or the generator panicked)"));
                    continue;
                };
                let debug = cap(format!("{:?}", input.value), limit);
                if debug != entry.debug {
                    self.report.regressions.stale += 1;
                    self.report.warn(
                        "regressions",
                        format!(
                            "stored input {point} no longer matches the generator (stored {}, regenerated {debug}); \
                             delete the entry or rerun the property",
                            entry.debug
                        ),
                    );
                    continue;
                }
                let at = input.at("regression", self.report.seed);
                let result = self.run_property(input.value.clone(), at).result;
                if Signature::of(&result).is_some() {
                    self.pending = Some(Found { input, result });
                    return true;
                }
            }
        }
        false
    }

    /// Render an input kept by provenance; `None` when it can no longer be
    /// regenerated.
    fn render_provenance(&mut self, source: Provenance) -> Option<String> {
        let limit = self.config.report.truncate_debug_at;
        match source {
            Provenance::Debug(debug) => Some(debug),
            Provenance::Point(point) => {
                self.regenerate(point).map(|value| cap(format!("{value:?}"), limit))
            },
        }
    }

    /// The targeted phase: mutate the best-scoring inputs to push the score
    /// up, looking for a failure along the way.
    fn targeted_search(&mut self) {
        if self.pool.is_empty() {
            return;
        }
        let target_tests = self.config.target.tests;
        let explore = self.config.target.explore;
        let mut iterations = 0u64;
        let mut discarded = 0u64;

        while iterations < target_tests {
            if self.timed_out_or_gave_up(discarded, target_tests) {
                break;
            }
            iterations += 1;
            let iteration = self.next_iteration();
            let input = if u01(&mut self.rng) < explore {
                let point = ReplayPoint { seed: self.report.seed, iteration };
                match self.draw_at(point, crate::report::Origin::Explore) {
                    Some(input) => input,
                    None if self.report.status != Status::Passed => break,
                    None => {
                        discarded += 1;
                        continue;
                    },
                }
            } else {
                let Some(seed) = self.pool.pick(&mut self.rng) else { break };
                match self.mutate_at(&seed.value, iteration) {
                    Some(input) => input,
                    None if self.report.status != Status::Passed => break,
                    None => continue,
                }
            };
            let tested = self.test(&input);
            if tested.failed() {
                break;
            }
            if tested.discarded() {
                discarded += 1;
                continue;
            }
            self.keep(input.value, &tested);
        }

        if let Some(summary) = self.target_summary(iterations) {
            self.report.target = Some(summary);
        }
    }

    /// Handle the pending failure: check it reproduces, shrink it (unless
    /// the loop already did), remember it, and fill in the report.
    fn fail(&mut self) {
        let limit = self.config.report.truncate_debug_at;
        let could_shrink = self.reproduce_pending();
        let Some(found) = self.pending.take() else {
            return;
        };
        let Some(mut shrinking) = self.shrinking.take() else {
            return;
        };
        let at = found.derived("shrink", self.report.seed);
        let Found { input, result } = found;
        let (kind, mut message, mut location) = details(result);
        let original = counterexample(&input.value, limit);
        let flaky = shrinking.flaky;

        if could_shrink
            && self.config.shrink.enabled
            && !shrinking.done
            && shrinking.stats.steps == 0
        {
            self.drive_shrink(&mut shrinking, at);
        }
        let mut shrunk = None;
        let mut shrink_stats = None;
        let minimal = shrinking.minimal.clone();
        if shrinking.done || shrinking.stats.steps > 0 {
            if let Some((shrunk_message, shrunk_location)) = shrinking.failure {
                shrunk = Some(counterexample(&minimal, limit));
                message = shrunk_message;
                location = shrunk_location;
            }
            shrink_stats = Some(shrinking.stats);
        }
        #[cfg(feature = "coverage")]
        if self.exec.coverage.spectrum.is_some() && !flaky {
            // Passing and failing neighbours of the minimal input sharpen the
            // spectrum where it matters.
            let neighbourhood = mix(self.report.seed, u64::MAX - 3);
            for i in 0..self.config.locate.neighbourhood as u64 {
                let mut source = Source::new(mix(neighbourhood, i), input.size);
                let mutant = self.generator.mutate(&minimal, &mut source);
                if source.discarded().is_none() {
                    self.run_property(mutant, At { origin: "neighbourhood", ..at });
                }
            }
            let panic = location.as_ref().map(|l| (l.file.as_str(), l.line));
            self.report.locate =
                self.exec.coverage.spectrum.as_ref().map(|s| s.rank(panic, self.config.locate.top));
        }
        // Like Hypothesis's "minimal failing case", the sinks end with the
        // shrunk input, run once more so Tyche lists it first.
        if self.exec.observing() && !flaky {
            self.run_property(minimal.clone(), At { origin: "minimal", ..at });
        }

        #[cfg(feature = "json")]
        if self.config.regressions.save
            && !flaky
            && let Some(point) = input.point
        {
            let entry = crate::regressions::Entry::new(
                &self.report.name,
                point.seed,
                point.iteration,
                original.debug.clone(),
                shrunk.as_ref().map(|c| c.debug.clone()),
            );
            match crate::regressions::save(&self.config.regressions.dir, &entry) {
                Ok((path, appended)) => {
                    self.report.regressions.file = Some(path);
                    self.report.regressions.saved = appended;
                },
                Err(e) => {
                    self.report.warn(
                        "regressions",
                        format!("could not write {}: {e}", self.config.regressions.dir.display()),
                    )
                },
            }
        }

        self.report.status = Status::Failed;
        self.report.failure = Some(Failure {
            kind,
            message,
            location,
            original,
            size: input.size,
            iteration: input.iteration,
            shrunk,
            replay: input.point,
            flaky,
            shrink: shrink_stats,
        });
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
        self.report.duration = self.started.elapsed();
        self.report.distinct_inputs_ratio =
            if self.generated == 0 { 1.0 } else { self.seen.len() as f64 / self.generated as f64 };
        self.report.unique_tests = self.stats.passed_seen.len() as u64;
        self.report.distinct_growth = std::mem::take(&mut self.stats.distinct_growth);
        for (name, values) in std::mem::take(&mut self.stats.features) {
            if !values.is_empty() {
                self.report.features.insert(name, crate::report::FeatureSummary::of(values));
            }
        }
        // Quantiles before any regeneration below adds to the timings.
        let prop = crate::report::DurationStats::of(std::mem::take(&mut self.stats.prop_micros));
        let generation =
            crate::report::DurationStats::of(std::mem::take(&mut self.stats.gen_micros));
        if let Some(prop) = prop {
            let zero = std::time::Duration::ZERO;
            let generation = generation.unwrap_or(crate::report::DurationStats {
                median: zero,
                p90: zero,
                max: zero,
            });
            let slowest =
                self.stats.slowest.take().and_then(|(_, source)| self.render_provenance(source));
            self.report.timing = Some(crate::report::Timing { prop, generation, slowest });
        }
        for candidate in self.stats.samples.drain() {
            if let Some(debug) = self.render_provenance(candidate.source) {
                self.report.samples.push(crate::report::Sample {
                    debug: cap(debug, SAMPLE_CHARS),
                    origin: candidate.origin,
                    size: candidate.size,
                    iteration: candidate.iteration,
                });
            }
        }
        for (label, (met, total)) in &self.covers {
            let share = if *total == 0 { 0.0 } else { *met as f64 / *total as f64 };
            self.report.covers.insert(label.clone(), share);
            if *total >= 20 && share < self.config.coverage.min_cover {
                self.report.warn(
                    "cover",
                    format!(
                        "cover `{label}` was met by {:.1}% of passing tests, below the required {:.0}%",
                        share * 100.0,
                        self.config.coverage.min_cover * 100.0
                    ),
                );
            }
        }
        #[cfg(feature = "coverage")]
        {
            if let Some(tracker) = self.exec.coverage.tracker.take() {
                let mut summary = tracker.summary();
                if let Some(targets) = self.exec.coverage.targets.take() {
                    summary.targets = targets.outcomes();
                    for outcome in &summary.targets {
                        if outcome.functions.is_empty() || outcome.hits > 0 {
                            continue;
                        }
                        let function = outcome.functions.join(", ");
                        let how = if outcome.function_inputs == 0 {
                            format!("`{function}` was never entered")
                        } else {
                            format!(
                                "`{function}` was entered by {} inputs that took other branches",
                                outcome.function_inputs
                            )
                        };
                        self.report.warn(
                            "coverage_target_missed",
                            format!("target {} was never reached: {how}", outcome.target),
                        );
                    }
                }
                if summary.inputs >= 100 && !summary.partial.is_empty() {
                    let since = summary.inputs - 1 - summary.last_novelty_at;
                    let missed: usize =
                        summary.partial.iter().map(|(_, hit, total)| total - hit).sum();
                    if since > (summary.inputs / 4).max(100) {
                        self.report.warn(
                            "coverage_plateau",
                            format!(
                                "coverage of `{}` last grew at input {} and {missed} {} in the functions it \
                                 reaches never hit; the generator may be too narrow",
                                summary.filter,
                                summary.last_novelty_at,
                                if missed == 1 { "counter was" } else { "counters were" }
                            ),
                        );
                    }
                }
                #[cfg(feature = "json")]
                if let (Some(grid), Some(dir)) =
                    (self.exec.coverage.grid.take(), self.config.coverage.html.as_ref())
                {
                    let samples: Vec<(String, String, usize)> = self
                        .report
                        .samples
                        .iter()
                        .map(|s| (s.debug.clone(), s.origin.to_string(), s.size))
                        .collect();
                    let html = grid.render(
                        tracker.selected(),
                        &crate::coverage::grid::Facts {
                            property: &self.report.name,
                            seed: self.report.seed,
                            tests: self.report.tests,
                            filter: &summary.filter,
                            inputs: summary.inputs,
                        },
                        &summary.targets,
                        &samples,
                    );
                    match crate::write_named(dir, &self.report.name, "html", html.as_bytes()) {
                        Ok(path) => self.report.coverage_html = Some(path),
                        Err(e) => {
                            self.report.warn(
                                "coverage_html",
                                format!(
                                    "could not write the coverage grid under {}: {e}",
                                    dir.display()
                                ),
                            )
                        },
                    }
                }
                self.report.coverage = Some(summary);
            }
            if let Some(dir) = &self.config.coverage.profraw_dir
                && crate::coverage::available()
            {
                match crate::coverage::tracker::write_profraw(dir, &self.report.name) {
                    Ok(path) => self.report.profraw = Some(path),
                    Err(e) => {
                        self.report.warn(
                            "profraw",
                            format!("could not write profraw to {}: {e}", dir.display()),
                        )
                    },
                }
            }
        }
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

/// Identifies "the same failure" while shrinking: the kind, and for panics
/// the site. Messages are ignored because they usually embed the input.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Signature {
    kind: FailureKind,
    panic_site: Option<(String, u32)>,
}

impl Signature {
    fn of(result: &Result<Outcome, PanicInfo>) -> Option<Self> {
        match result {
            Ok(Outcome::Pass) | Ok(Outcome::Discard(_)) => None,
            Ok(Outcome::False) => Some(Signature { kind: FailureKind::False, panic_site: None }),
            Ok(Outcome::Fail(_)) => Some(Signature { kind: FailureKind::Error, panic_site: None }),
            Err(info) => {
                Some(Signature {
                    kind: FailureKind::Panic,
                    panic_site: info.location.as_ref().map(|l| (l.file.clone(), l.line)),
                })
            },
        }
    }
}

/// Kind, message and location of a failing result.
fn details(result: Result<Outcome, PanicInfo>) -> (FailureKind, Option<String>, Option<Location>) {
    match result {
        Ok(Outcome::Fail(message)) => (FailureKind::Error, Some(message), None),
        Err(info) => (FailureKind::Panic, Some(info.message), info.location),
        Ok(_) => (FailureKind::False, None, None),
    }
}

/// The message and location of `result` when it fails like `signature`.
fn same_failure(result: Result<Outcome, PanicInfo>, signature: &Signature) -> shrink::SameFailure {
    if Signature::of(&result).as_ref() != Some(signature) {
        return None;
    }
    let (_, message, location) = details(result);
    Some((message, location))
}

/// Run the property once with panic capture and the side channel active.
fn evaluate<T, R: Testable, P: FnMut(T) -> R>(
    property: &mut P,
    value: T,
    trace_observations: bool,
) -> (Result<Outcome, PanicInfo>, labels::Observed) {
    labels::collect(trace_observations, || panic::catch(|| property(value).into_outcome()))
}

/// `value`'s `Debug` rendering, cut at `limit` characters.
fn counterexample<T: Debug>(value: &T, limit: usize) -> Counterexample {
    let debug = format!("{value:?}");
    Counterexample { truncated: debug.chars().nth(limit).is_some(), debug: cap(debug, limit) }
}

/// Hash a value's `Debug` rendering without allocating it.
fn debug_hash<T: Debug>(value: &T) -> u64 {
    struct HashWriter(DefaultHasher);
    impl fmt::Write for HashWriter {
        fn write_str(&mut self, s: &str) -> fmt::Result {
            self.0.write(s.as_bytes());
            Ok(())
        }
    }
    let mut writer = HashWriter(DefaultHasher::new());
    let _ = write!(writer, "{value:?}");
    writer.0.finish()
}

/// The size parameter for generation attempt `iteration`: `floor(log2(i + 1))`,
/// capped at `max_size`. This spends twice as many attempts at each successive
/// size while remaining a pure function of the iteration for replay.
fn size_for(iteration: u64, max_size: usize) -> usize {
    let size = u64::BITS - iteration.saturating_add(1).leading_zeros() - 1;
    (size as usize).min(max_size)
}

/// A fresh seed from the operating system's randomness (via `RandomState`).
fn random_seed() -> u64 {
    RandomState::new().hash_one(Instant::now())
}

/// Derive an independent 64-bit value from `seed` and `index` (SplitMix64).
pub(crate) fn mix(seed: u64, index: u64) -> u64 {
    let mut z = seed
        .wrapping_add(index.wrapping_mul(0x9E37_79B9_7F4A_7C15))
        .wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use {
        super::*,
        crate::{
            GenExt,
            classify,
            gens,
        },
    };

    fn cfg() -> Config {
        Config::default().seed(1).tests(64).no_regressions()
    }

    #[test]
    fn mutation_time_counts_as_generation_time() {
        let slow = gens::int(0i32..=100).with_mutate(|value, _| {
            std::thread::sleep(std::time::Duration::from_millis(2));
            *value
        });
        let mut config = cfg().tests(20);
        config.target.tests = 20;
        let report = check(&config, slow, |x| {
            crate::target(f64::from(x));
            true
        });
        let iterations = report.target.as_ref().expect("the targeted phase ran").iterations;
        // Most targeted iterations mutate (the rest explore), each for 2 ms.
        assert!(
            report.gen_time >= std::time::Duration::from_millis(iterations),
            "{:?} over {iterations} targeted iterations",
            report.gen_time
        );
    }

    #[test]
    fn passing_property() {
        let report = check(&cfg(), gens::vec(gens::int(0..=9)), |v: Vec<i32>| v.len() <= 100);
        assert_eq!(report.status, Status::Passed);
        assert_eq!(report.tests, 64);
        assert_eq!(report.discarded, 0);
        assert_eq!(report.iterations, 64);
        assert_eq!(report.sizes.keys().copied().collect::<Vec<_>>(), [0, 1, 2, 3, 4, 5, 6]);
        assert!(report.passed());
        assert!(report.distinct_inputs_ratio >= 0.5, "{}", report.distinct_inputs_ratio);
        // An instrumented build run with several test threads adds
        // `coverage_threads`, which says nothing about the property.
        assert!(report.health.iter().all(|h| h.kind == "coverage_threads"), "{:?}", report.health);
        report.assert_passed();
        assert!(report.to_string().starts_with("ephyra: ok, 64 tests passed"));
    }

    #[test]
    fn false_result_is_a_shrunk_counterexample() {
        let report = check(&cfg(), gens::vec(gens::int(0..=9)), |v: Vec<i32>| v.len() < 3);
        assert_eq!(report.status, Status::Failed);
        let failure = report.failure.as_ref().unwrap();
        assert_eq!(failure.kind, FailureKind::False);
        assert!(failure.original.debug.starts_with('['));
        assert!(!failure.flaky);
        assert_eq!(failure.iteration + 1, report.iterations);
        assert_eq!(failure.counterexample().debug, "[0, 0, 0]");
        assert!(failure.shrink.is_some());
        let point = failure.replay.unwrap();
        assert_eq!(point.seed, 1);
        let text = report.to_string();
        assert!(text.contains("property returned false"), "{text}");
        assert!(text.contains("counterexample: [0, 0, 0]"), "{text}");
        assert!(text.contains(&format!("EPHYRA_REPLAY={point}")), "{text}");
    }

    #[test]
    fn replay_point_reproduces_without_random_search() {
        let property = |v: Vec<i32>| v.len() < 3;
        let first = check(&cfg(), gens::vec(gens::int(0..=9)), property);
        let point = first.failure.as_ref().unwrap().replay.unwrap();

        let second = check(&cfg().seed(999).replay(point), gens::vec(gens::int(0..=9)), property);
        assert_eq!(second.status, Status::Failed);
        assert_eq!(second.iterations, 0, "no random search was needed");
        assert_eq!(
            second.failure.as_ref().unwrap().original,
            first.failure.as_ref().unwrap().original
        );
        assert!(second.replay.as_ref().unwrap().reproduced);
        assert!(second.to_string().contains("reproduced the failure"));

        let fixed = check(&cfg().replay(point), gens::vec(gens::int(0..=9)), |_: Vec<i32>| true);
        assert_eq!(fixed.status, Status::Passed);
        assert!(!fixed.replay.as_ref().unwrap().reproduced);
        assert_eq!(fixed.tests, 64);
    }

    #[cfg(feature = "json")]
    #[test]
    fn regressions_are_saved_and_replayed() {
        let dir =
            std::env::temp_dir().join(format!("ephyra-runner-regressions-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let config = Config::default().seed(5).tests(64).name("sum_prop").regressions_dir(&dir);
        let property = |v: Vec<i32>| v.iter().sum::<i32>() < 10;

        let first = check(&config, gens::vec(gens::int(0..=9)), property);
        assert_eq!(first.status, Status::Failed);
        assert!(first.regressions.saved);
        let file = first.regressions.file.clone().unwrap();
        assert!(file.ends_with("sum_prop.jsonl"));
        assert!(first.to_string().contains("remembered in"));

        let second = check(&config.clone().seed(6), gens::vec(gens::int(0..=9)), property);
        assert_eq!(second.status, Status::Failed);
        assert_eq!(second.regressions.replayed, 1);
        assert_eq!(second.iterations, 0);
        assert_eq!(second.counterexample().unwrap().debug, first.counterexample().unwrap().debug);
        assert!(!second.regressions.saved, "the same point is not stored twice");

        let fixed = check(&config, gens::vec(gens::int(0..=9)), |_: Vec<i32>| true);
        assert_eq!(fixed.status, Status::Passed);
        assert_eq!(fixed.regressions.replayed, 1);

        let changed = check(&config, gens::vec(gens::int(100..=109)), |_: Vec<i32>| true);
        assert_eq!(changed.regressions.stale, 1);
        assert!(changed.health.iter().any(|w| w.kind == "regressions"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(feature = "json")]
    #[test]
    fn json_report_file_is_appended() {
        let dir = std::env::temp_dir().join(format!("ephyra-runner-json-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("reports.jsonl");
        let mut config = cfg();
        config.report.json_path = Some(path.clone());
        check(&config, gens::int(0..=9), |_: i32| true);
        check(&config, gens::int(0..=9), |x: i32| x < 5);
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<serde_json::Value> =
            text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["status"], "Passed");
        assert_eq!(lines[1]["failure"]["shrunk"]["debug"], "5");
        assert!(lines[1]["failure"]["replay"]["iteration"].is_number());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn env_problems_become_warnings() {
        let (config, problems) =
            cfg().apply_env_vars(vec![("EPHYRA_BOGUS".to_string(), "1".to_string())]);
        assert_eq!(problems.len(), 1);
        let report = check(&config, gens::int(0..=9), |_: i32| true);
        assert!(report.passed());
    }

    #[test]
    fn health_warnings_for_discards_and_degenerate_generators() {
        let report = check(&cfg(), gens::int(0..=9), |x: i32| {
            crate::assume!(x == 0);
            TestResult::Pass
        });
        assert!(report.health.iter().any(|w| w.kind == "discard_ratio"), "{:?}", report.health);
        let report = check(&cfg(), gens::constant(1), |_: i32| true);
        assert!(report.distinct_inputs_ratio < 0.1);
        assert!(
            report.health.iter().any(|w| w.kind == "low_distinct_inputs"),
            "{:?}",
            report.health
        );
    }

    #[test]
    fn error_and_test_result_failures_carry_messages() {
        let report = check(&cfg(), gens::int(0..=9), |x: i32| {
            if x > 5 { Err(format!("too big: {x}")) } else { Ok(()) }
        });
        let failure = report.failure.as_ref().unwrap();
        assert_eq!(failure.kind, FailureKind::Error);
        assert_eq!(
            failure.message.as_deref(),
            Some("\"too big: 6\""),
            "message reflects the shrunk input"
        );
        assert_eq!(failure.counterexample().debug, "6");

        let report =
            check(&cfg(), gens::int(0..=9), |x: i32| TestResult::check(x < 5, "x must be small"));
        let failure = report.failure.as_ref().unwrap();
        assert_eq!(failure.kind, FailureKind::Error);
        assert_eq!(failure.message.as_deref(), Some("x must be small"));
        assert_eq!(failure.counterexample().debug, "5");
    }

    #[test]
    fn panics_are_captured_with_location() {
        let report = check(&cfg(), gens::int(0..=9), |x: i32| {
            if x > 3 {
                panic!("x was {x}");
            }
        });
        let failure = report.failure.as_ref().unwrap();
        assert_eq!(failure.kind, FailureKind::Panic);
        assert_eq!(failure.message.as_deref(), Some("x was 4"));
        let location = failure.location.as_ref().expect("panic location");
        assert!(location.file.ends_with("runner/mod.rs"), "{}", location.file);
        assert!(report.to_string().contains("property panicked: x was 4"));
    }

    #[test]
    fn shrinking_keeps_the_failure_signature() {
        let report = check(&cfg().seed(3), gens::int(0..=100), |x: i32| {
            if x > 50 {
                panic!("large");
            }
            x <= 10
        });
        let failure = report.failure.as_ref().unwrap();
        match failure.kind {
            FailureKind::Panic => assert_eq!(failure.counterexample().debug, "51"),
            FailureKind::False => assert_eq!(failure.counterexample().debug, "11"),
            FailureKind::Error => unreachable!(),
        }
    }

    #[test]
    fn attached_shrinker_is_used_and_filtered() {
        let generator =
            gens::int(0i32..=100).filter("even", |x| x % 2 == 0).with_shrink(|x: &i32, visitor| {
                visitor(x - 2)?;
                visitor(x / 2)
            });
        let report = check(&cfg(), generator, |x: i32| x < 11);
        let failure = report.failure.as_ref().unwrap();
        assert_eq!(failure.counterexample().debug, "12", "{report}");
        assert!(failure.shrink.as_ref().unwrap().steps > 0);
    }

    #[test]
    fn shrinking_can_be_disabled() {
        let report =
            check(&cfg().no_shrink(), gens::vec(gens::int(0..=9)), |v: Vec<i32>| v.len() < 3);
        let failure = report.failure.as_ref().unwrap();
        assert!(failure.shrunk.is_none());
        assert!(failure.shrink.is_none());
        assert_eq!(failure.counterexample(), &failure.original);
    }

    #[test]
    fn shrink_budget_is_reported() {
        let config = Config {
            shrink: crate::config::ShrinkConfig { max_steps: 3, ..Default::default() },
            ..cfg()
        };
        let report = check(&config, gens::vec(gens::int(0..=1000)).len(10..=20), |v: Vec<i32>| {
            v.iter().sum::<i32>() < 100
        });
        let failure = report.failure.as_ref().unwrap();
        assert!(failure.shrink.as_ref().unwrap().budget_exhausted);
        assert!(report.health.iter().any(|w| w.kind == "shrink_budget"));
        assert!(report.to_string().contains("budget exhausted"));
    }

    #[test]
    fn discards_from_property_and_generator_are_counted() {
        let report = check(&cfg(), gens::int(0..=9), |x: i32| {
            crate::assume!(x % 2 == 0);
            TestResult::Pass
        });
        assert_eq!(report.status, Status::Passed);
        assert_eq!(report.tests, 64);
        assert!(report.discarded > 0);
        assert_eq!(
            report.discard_reasons.keys().next().map(String::as_str),
            Some("assume: x % 2 == 0")
        );

        let report =
            check(&cfg(), gens::int(0i32..=9).filter("even", |x| x % 2 == 0), |_: i32| true);
        assert_eq!(report.status, Status::Passed);
        assert_eq!(report.discarded, 0, "filter retries do not discard unless exhausted");
        let report = check(
            &cfg().filter_max_attempts(2),
            gens::int(0i32..=9).filter("never", |_| false),
            |_: i32| true,
        );
        assert_eq!(report.status, Status::GaveUp);
        assert_eq!(report.discard_reasons.get("filter:never").copied(), Some(report.discarded));
    }

    #[test]
    fn gives_up_when_everything_is_discarded() {
        let report = check(&cfg(), gens::int(0..=9), |_: i32| TestResult::discard("nope"));
        assert_eq!(report.status, Status::GaveUp);
        assert_eq!(report.tests, 0);
        assert!(report.discarded > 64 * 10);
        assert!(report.to_string().starts_with("ephyra: GAVE UP"));
    }

    #[test]
    fn times_out() {
        let config = cfg().tests(1_000_000).timeout(std::time::Duration::from_millis(20));
        let report = check(&config, gens::int(0..=9), |_: i32| {
            std::thread::sleep(std::time::Duration::from_millis(1));
            true
        });
        assert_eq!(report.status, Status::TimedOut);
        assert!(report.tests < 1_000_000);
    }

    #[test]
    fn time_budget_stops_the_random_phase() {
        let config = cfg().tests(1_000_000).budget(std::time::Duration::from_millis(20));
        let report = check(&config, gens::int(0..=9), |_: i32| {
            std::thread::sleep(std::time::Duration::from_millis(1));
            true
        });
        assert_eq!(report.status, Status::Passed);
        assert!(report.stopped_by_budget);
        assert!(report.tests < 1_000_000);
        assert!(report.to_string().contains("stopped by the time budget"));
    }

    #[test]
    fn generator_panic_aborts() {
        let report =
            check(&cfg(), |_: &mut Source| -> i32 { panic!("generator bug") }, |_: i32| true);
        assert_eq!(report.status, Status::Aborted);
        let abort = report.abort.as_ref().unwrap();
        assert_eq!(abort.message, "generator bug");
        assert!(abort.location.is_some());
        assert!(report.to_string().contains("ABORTED"));
    }

    #[test]
    fn labels_are_reported_for_passing_tests() {
        let report = check(&cfg(), gens::vec(gens::int(0..=9)), |v: Vec<i32>| {
            classify(v.is_empty(), "empty");
            classify(!v.is_empty(), "non-empty");
            true
        });
        let empty = report.labels.get("empty").copied().unwrap_or(0);
        let non_empty = report.labels.get("non-empty").copied().unwrap_or(0);
        assert_eq!(empty + non_empty, 64);
        assert!(report.to_string().contains("labels: empty"));
    }

    #[test]
    fn same_seed_same_report() {
        let run =
            || check(&cfg(), gens::vec(gens::int(0..=9)), |v: Vec<i32>| v.iter().sum::<i32>() < 20);
        let a = run();
        let b = run();
        assert_eq!(a.status, Status::Failed);
        let fa = a.failure.as_ref().unwrap();
        let fb = b.failure.as_ref().unwrap();
        assert_eq!(fa.original, fb.original);
        assert_eq!(fa.shrunk, fb.shrunk);
        assert_eq!(fa.replay, fb.replay);
        assert_eq!(a.tests, b.tests);
        let c = check(&cfg().seed(2), gens::vec(gens::int(0..=9)), |v: Vec<i32>| {
            v.iter().sum::<i32>() < 20
        });
        assert_ne!(a.seed, c.seed);
    }

    #[test]
    fn unseeded_runs_get_distinct_seeds() {
        let a = check(&Config::default().tests(1).no_regressions(), gens::bool(), |_| true);
        let b = check(&Config::default().tests(1).no_regressions(), gens::bool(), |_| true);
        assert_ne!(a.seed, b.seed);
    }

    #[test]
    fn flaky_property_is_flagged_and_not_shrunk() {
        let calls = Cell::new(0);
        let report = check(&cfg(), gens::int(0..=9), |_: i32| {
            calls.set(calls.get() + 1);
            calls.get() != 3
        });
        let failure = report.failure.as_ref().unwrap();
        assert!(failure.flaky);
        assert!(failure.shrunk.is_none());
        assert!(report.health.iter().any(|h| h.kind == "flaky"), "{:?}", report.health);
        assert!(report.to_string().contains("FLAKY"));
    }

    #[test]
    fn debug_output_is_truncated() {
        let config = Config {
            report: crate::config::ReportConfig { truncate_debug_at: 10, ..Default::default() },
            ..cfg().no_shrink()
        };
        let report = check(&config, gens::vec(gens::int(0..=9)).len(50..=50), |_: Vec<i32>| false);
        let original = &report.failure.as_ref().unwrap().original;
        assert!(original.truncated);
        assert_eq!(original.debug.chars().count(), 11);
        assert!(report.to_string().contains("(truncated)"));
    }

    #[test]
    fn concurrent_checks_keep_their_own_panics() {
        let handles: Vec<_> = (0..6)
            .map(|i| {
                std::thread::spawn(move || {
                    let report = check(&cfg(), gens::int(0..=9), move |x: i32| {
                        if x > 2 {
                            panic!("worker {i}");
                        }
                    });
                    let failure = report.failure.unwrap();
                    assert_eq!(failure.message.as_deref(), Some(format!("worker {i}").as_str()));
                    assert_eq!(failure.counterexample().debug, "3");
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
    }

    #[test]
    fn targeted_search_finds_what_random_search_misses() {
        let generator = || gens::vec(gens::int(0u8..=255)).len(8..=8);
        let property = |v: Vec<u8>| {
            let sum: u32 = v.iter().map(|&x| u32::from(x)).sum();
            crate::target(f64::from(sum));
            sum < 1900
        };
        let mut random_only = cfg().seed(11);
        random_only.target.tests = 0;
        let report = check(&random_only, generator(), property);
        assert_eq!(report.status, Status::Passed, "{report}");
        let summary = report.target.as_ref().expect("scores were observed");
        assert_eq!(summary.iterations, 0);
        assert!(summary.best_score < 1900.0);

        let mut targeted = cfg().seed(11);
        targeted.target.tests = 2000;
        let report = check(&targeted, generator(), property);
        assert_eq!(report.status, Status::Failed, "{report}");
        let failure = report.failure.as_ref().unwrap();
        assert!(failure.replay.is_none(), "mutants replay through the run seed");
        let shrunk: Vec<u32> = failure
            .counterexample()
            .debug
            .trim_matches(|c| c == '[' || c == ']')
            .split(", ")
            .map(|x| x.parse().unwrap())
            .collect();
        assert_eq!(shrunk.iter().sum::<u32>(), 1900, "{shrunk:?}");
        assert!(report.to_string().contains("found in the targeted phase"));

        let again = check(&targeted, generator(), property);
        assert_eq!(again.failure.as_ref().unwrap().original, failure.original, "deterministic");
    }

    #[test]
    fn target_summary_reports_the_best_input() {
        let generator = gens::vec(gens::int(0..=9)).len(4..=4);
        let property = |v: Vec<i32>| {
            crate::target(f64::from(v.iter().sum::<i32>()));
            true
        };
        let mut random_only = cfg().seed(4);
        random_only.target.tests = 0;
        let baseline = check(&random_only, generator.clone(), property);
        let baseline_best = baseline.target.as_ref().unwrap().best_score;

        let mut config = cfg().seed(4);
        config.target.tests = 300;
        let report = check(&config, generator.clone(), property);
        assert!(report.passed());
        let summary = report.target.as_ref().unwrap();
        assert!(summary.best_score > baseline_best, "{} vs {baseline_best}", summary.best_score);
        assert!(summary.best_score >= 30.0, "{}", summary.best_score);
        assert_eq!(summary.best_debug.matches(',').count(), 3, "{}", summary.best_debug);
        assert_eq!(summary.iterations, 300);
        assert!(summary.improvements > 0);
        assert!(report.to_string().contains("target: best score"));

        let plain = check(&config, generator, |_: Vec<i32>| true);
        assert!(plain.target.is_none(), "no scores, no targeted phase");
        assert_eq!(plain.iterations, 64);
    }

    #[test]
    fn features_timing_and_samples_are_reported() {
        let mut config = cfg();
        config.tests = 200;
        let report = check(&config, gens::vec(gens::int(0i32..=9)), |v: Vec<i32>| {
            crate::feature("len", v.len());
            crate::feature("parity", if v.len().is_multiple_of(2) { "even" } else { "odd" });
            crate::target(v.iter().sum::<i32>() as f64);
            true
        });
        assert_eq!(report.status, Status::Passed, "{report}");
        let len = &report.features["len"];
        assert_eq!(len.count, report.tests, "the targeted phase's passes count too");
        assert!(len.min <= len.median && len.median <= len.p90 && len.p90 <= len.max);
        assert_eq!(len.histogram.iter().map(|b| b.count).sum::<u64>(), report.tests, "{len:?}");
        assert!(report.features.contains_key("target"));
        assert_eq!(
            report.labels.get("parity=even").copied().unwrap_or(0)
                + report.labels.get("parity=odd").copied().unwrap_or(0),
            report.tests
        );
        assert!(report.unique_tests <= report.tests && report.unique_tests > 100, "{report}");
        assert!(!report.distinct_growth.points.is_empty());
        let timing = report.timing.as_ref().expect("inputs ran");
        assert!(timing.prop.median <= timing.prop.p90 && timing.prop.p90 <= timing.prop.max);
        assert!(timing.slowest.is_some());
        assert!(!report.samples.is_empty() && report.samples.len() <= 5, "{report}");
        let smallest = &report.samples[0];
        assert!(report.samples.iter().all(|s| s.size >= smallest.size));
        let text = report.to_string();
        assert!(text.contains("feature len:") && text.contains("histogram:"), "{text}");
        assert!(text.contains("timing: property median") && text.contains("sample ("), "{text}");
        assert!(text.contains("distinct,"), "{text}");
    }

    #[cfg(feature = "json")]
    #[test]
    fn observation_files_follow_openpbtstats() {
        let dir = std::env::temp_dir().join(format!("ephyra-observed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut config = cfg().name("observed");
        config.tests = 50;
        config.report.observations = Some(dir.clone());
        let generator = (|source: &mut Source| vec![9; source.size().min(5)]).with_shrink(
            |value: &Vec<i32>, visitor| {
                if value.iter().any(|&x| x != 1) {
                    visitor(vec![1; value.len()])?;
                }
                std::ops::ControlFlow::Continue(())
            },
        );
        let report = check(&config, generator, |v: Vec<i32>| {
            crate::feature("len", v.len());
            crate::observe("elements", || v.len());
            crate::label("seen");
            if v.len() == 4 {
                crate::TestResult::discard("four")
            } else {
                crate::TestResult::check(v.len() < 5, "too long")
            }
        });
        assert_eq!(report.status, Status::Failed, "{report}");
        let text = std::fs::read_to_string(dir.join("observed.jsonl")).expect("file written");
        let _ = std::fs::remove_dir_all(&dir);
        let lines: Vec<serde_json::Value> =
            text.lines().map(|l| serde_json::from_str(l).expect("valid json")).collect();
        let info = lines.last().unwrap();
        assert_eq!(info["type"], "info");
        assert_eq!(info["title"], "ephyra report");
        assert!(info["content"].as_str().unwrap().contains("ephyra: FAILED"));
        let cases: Vec<&serde_json::Value> =
            lines.iter().filter(|l| l["type"] == "test_case").collect();
        assert!(
            cases
                .iter()
                .all(|c| c["run_start"] == info["run_start"] && c["property"] == "observed")
        );
        let count = |status: &str, origin: &str| {
            cases.iter().filter(|c| c["status"] == status && c["how_generated"] == origin).count()
                as u64
        };
        assert_eq!(count("passed", "random"), report.tests);
        assert_eq!(count("gave_up", "random"), report.discarded);
        assert_eq!(count("failed", "random"), 1);
        assert!(count("passed", "shrink") + count("failed", "shrink") > 0, "shrink candidates");
        let shrink_cases: Vec<_> =
            cases.iter().filter(|case| case["how_generated"] == "shrink").collect();
        assert!(shrink_cases.iter().all(|case| case["metadata"]["shrink_accepted"].is_boolean()));
        assert!(
            shrink_cases.iter().any(|case| case["metadata"]["shrink_accepted"] == true),
            "at least one shrink candidate should be accepted"
        );
        let last_case = cases.last().unwrap();
        assert_eq!(last_case["how_generated"], "minimal");
        assert_eq!(last_case["status"], "failed");
        assert_eq!(
            last_case["representation"],
            report.failure.as_ref().unwrap().counterexample().debug
        );
        let passed = cases.iter().find(|c| c["status"] == "passed").unwrap();
        assert!(passed["representation"].as_str().unwrap().starts_with('['));
        assert!(passed["features"]["len"].is_number());
        assert_eq!(passed["features"]["seen"], "");
        assert!(passed["observations"]["elements"].is_number());
        assert!(passed["timing"]["execute:test"].is_number());
        assert!(passed["timing"]["generate:input"].is_number());
        assert_eq!(passed["metadata"]["seed"], report.seed);
        let failed = cases.iter().find(|c| c["status"] == "failed").unwrap();
        assert_eq!(failed["status_reason"], "too long");
        let discarded = cases.iter().find(|c| c["status"] == "gave_up").unwrap();
        assert_eq!(discarded["status_reason"], "four");
        #[cfg(feature = "coverage")]
        if crate::coverage::available() && crate::coverage::locate::Spectrum::new(&[]).is_some() {
            assert!(passed["coverage"].is_object(), "{passed}");
        } else {
            assert!(passed["coverage"].is_null(), "{passed}");
        }
        #[cfg(not(feature = "coverage"))]
        assert!(passed["coverage"].is_null(), "{passed}");
    }

    #[test]
    fn installed_sinks_see_the_runs_they_open() {
        use std::sync::{
            Arc,
            Mutex,
        };

        use crate::sink::{
            CaseStatus,
            RunInfo,
            RunSink,
            Sink,
        };

        #[derive(Default)]
        struct Seen {
            opened: Vec<(String, u64)>,
            cases: Vec<(CaseStatus, String, String, bool)>,
            finished: Option<Status>,
        }
        struct Recorder(Arc<Mutex<Seen>>);
        struct Recording(Arc<Mutex<Seen>>);
        impl Sink for Recorder {
            fn open(&self, run: &RunInfo<'_>) -> Option<Box<dyn RunSink>> {
                if !run.property.starts_with("sink-test") {
                    return None;
                }
                self.0.lock().unwrap().opened.push((run.property.to_string(), run.seed));
                Some(Box::new(Recording(self.0.clone())))
            }
        }
        impl RunSink for Recording {
            fn test_case(&mut self, case: &crate::sink::TestCase<'_>) {
                let observed = case.observations.iter().any(|(name, _)| name == "elements");
                self.0.lock().unwrap().cases.push((
                    case.status,
                    case.status_reason.to_string(),
                    case.origin.to_string(),
                    observed,
                ));
            }
            fn finish(&mut self, report: &Report) -> Result<(), String> {
                self.0.lock().unwrap().finished = Some(report.status);
                Err("the agent is away".to_string())
            }
        }

        let seen = Arc::new(Mutex::new(Seen::default()));
        crate::sink::install(Recorder(seen.clone()));
        let mut config = cfg().name("sink-test-failing");
        config.tests = 100;
        let report = check(&config, gens::int(0i32..=100), |n: i32| {
            crate::observe("elements", || n);
            crate::TestResult::check(n < 50, "too big")
        });
        assert_eq!(report.status, Status::Failed, "{report}");
        assert!(
            report
                .health
                .iter()
                .any(|w| w.kind == "observations" && w.message == "the agent is away"),
            "{report}"
        );
        let unseen = check(&cfg().name("other").tests(10), gens::int(0i32..=9), |_| true);
        assert!(unseen.passed());

        let seen = seen.lock().unwrap();
        assert_eq!(seen.opened, vec![("sink-test-failing".to_string(), report.seed)]);
        assert_eq!(seen.finished, Some(Status::Failed));
        let passes = seen.cases.iter().filter(|c| c.0 == CaseStatus::Passed && c.2 == "random");
        assert_eq!(passes.count() as u64, report.tests);
        assert!(seen.cases.iter().all(|c| c.3), "observe closures run while a sink is open");
        let last = seen.cases.last().unwrap();
        assert_eq!(
            (last.0, last.1.as_str(), last.2.as_str()),
            (CaseStatus::Failed, "too big", "minimal")
        );
    }

    #[test]
    fn samples_keep_the_smallest_the_largest_and_a_reservoir() {
        let mut samples = Samples::new(5);
        for (i, size) in [5usize, 3, 9, 1, 7, 8, 2, 6].into_iter().enumerate() {
            let r = hash_u01(1, i as u64);
            samples.consider(size, i as u64, crate::report::Origin::Random, r, || {
                Provenance::Debug(size.to_string())
            });
        }
        let kept = samples.drain();
        assert_eq!(kept[0].size, 1);
        assert_eq!(kept[1].size, 9);
        assert!(kept.len() <= 5 && kept.len() >= 3, "{}", kept.len());
        let mut none = Samples::new(0);
        none.consider(
            1,
            0,
            crate::report::Origin::Random,
            0.5,
            || Provenance::Debug(String::new()),
        );
        assert!(none.drain().is_empty());
    }

    #[test]
    fn cover_labels_are_reported_and_warned() {
        let report = check(&cfg(), gens::vec(gens::int(0..=9)), |v: Vec<i32>| {
            crate::cover(v.len() > 100, "long");
            crate::cover(true, "always");
            true
        });
        assert_eq!(report.covers.get("long"), Some(&0.0));
        assert_eq!(report.covers.get("always"), Some(&1.0));
        let warning = report.health.iter().find(|w| w.kind == "cover").expect("cover warning");
        assert!(warning.message.contains("`long`"), "{}", warning.message);
        assert!(report.to_string().contains("cover: always 100%, long 0%"), "{report}");
    }

    #[cfg(feature = "coverage")]
    #[test]
    fn the_default_coverage_filter_is_the_property_crate() {
        assert_eq!(crate_of("bst::main::{{closure}}"), "bst");
        assert_eq!(crate_of("&my_crate::props::sorted"), "my_crate");
        assert_eq!(crate_of("&mut my_crate::props::sorted"), "my_crate");
        assert_eq!(crate_of("ephyra"), "ephyra");
        assert_eq!(crate_of(super::crate_of_probe_name()), "ephyra");
    }

    #[cfg(all(ephyra_instrumented, feature = "coverage"))]
    const BRANCHY_LINE: u32 = line!() + 3;
    #[cfg(all(ephyra_instrumented, feature = "coverage"))]
    #[rustfmt::skip]
    fn branchy(x: i32) -> i32 {
        if x > 1000 {
            1
        } else {
            2
        }
    }

    #[cfg(all(ephyra_instrumented, feature = "coverage"))]
    #[test]
    fn coverage_targets_are_resolved_tracked_and_reported() {
        use crate::config::CoverageTarget;
        let then_branch = CoverageTarget::new(file!(), BRANCHY_LINE + 2..=BRANCHY_LINE + 2);
        let else_branch = CoverageTarget::new(file!(), BRANCHY_LINE + 4..=BRANCHY_LINE + 4);
        let nowhere = CoverageTarget::new("no/such/file.rs", 1..=1);
        let mut config = cfg();
        config.tests = 100;
        config.coverage.targets = vec![then_branch.clone(), else_branch.clone(), nowhere.clone()];
        config.coverage.targets_file = None;
        let report = check(&config, gens::int(-100i32..=100), |x| branchy(x) == 2);
        assert_eq!(report.status, Status::Passed, "{report}");
        let coverage = report.coverage.as_ref().expect("instrumented");
        assert_eq!(coverage.targets.len(), 3, "{report}");
        let (then, els, none) = (&coverage.targets[0], &coverage.targets[1], &coverage.targets[2]);
        assert_eq!(then.target, then_branch);
        assert!(then.functions.iter().any(|f| f.contains("branchy")), "{then:?}");
        assert_eq!(then.hits, 0, "{then:?}");
        assert!(then.function_inputs > 0, "{then:?}");
        assert_eq!(els.hits, els.function_inputs, "{els:?}");
        assert_eq!(els.first_hit_at, Some(1), "{els:?}");
        assert!(none.functions.is_empty() && none.target == nowhere, "{none:?}");
        let kinds: Vec<&str> = report.health.iter().map(|w| w.kind.as_str()).collect();
        assert!(kinds.contains(&"coverage_target_missed"), "{kinds:?}");
        assert!(kinds.contains(&"coverage_target"), "{kinds:?}");
        let text = report.to_string();
        assert!(text.contains("never reached") && text.contains("took other branches"), "{text}");
        assert!(text.contains("hit by"), "{text}");
        assert!(report.target.is_some(), "targets turn the guided search on: {report}");
    }

    #[cfg(all(ephyra_instrumented, feature = "coverage"))]
    #[test]
    fn novelty_counts_what_an_input_reached_first() {
        let mut config = cfg();
        config.seed = Some(1);
        config.coverage.enabled = true;
        let mut run = Run::new(&config, gens::int(0i32..=1), |x: i32| branchy(x) > 0);
        let input = run.draw().expect("ints never discard");
        let first = run.test(&input);
        assert!(first.novel() && first.novelty() > 0, "{first:?}");
        let again = run.test(&input);
        assert!(!again.novel(), "{again:?}");
        assert_eq!(again.novelty(), 0, "the same input reaches nothing new");
        let other = Input { value: 2000, ..input.clone() };
        let differs = run.test(&other);
        assert!(differs.novelty() > 0, "the other branch is new: {differs:?}");
        run.finish().assert_passed();
    }

    #[cfg(all(ephyra_instrumented, feature = "coverage", feature = "json"))]
    #[test]
    fn the_coverage_grid_is_written() {
        use crate::config::CoverageTarget;
        let dir = std::env::temp_dir().join(format!("ephyra-grid-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut config = cfg().name("grid::branchy");
        config.tests = 50;
        config.coverage.html = Some(dir.clone());
        config.coverage.targets =
            vec![CoverageTarget::new(file!(), BRANCHY_LINE + 2..=BRANCHY_LINE + 2)];
        config.coverage.targets_file = None;
        let report = check(&config, gens::int(-100i32..=100), |x| branchy(x) == 2);
        let page = report.coverage_html.clone().expect("grid written");
        assert_eq!(page, dir.join("grid_branchy.html"));
        assert!(report.to_string().contains("coverage grid written to"), "{report}");
        let html = std::fs::read_to_string(&page).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(html.contains("ephyra::runner::tests::branchy"), "function listed");
        assert!(html.contains("\"file_index\""), "targets embedded");
        assert!(html.contains("fn branchy(x: i32) -> i32 {"), "source embedded");
        // The page embeds this very file as source, so look at the script
        // tag rather than for the placeholder's absence.
        assert!(html.contains("<script id=\"data\" type=\"application/json\">{"), "data embedded");
        let start = html.find("<script id=\"data\"").unwrap();
        let end = html[start..].find("</script>").unwrap() + start;
        let json_text = &html[html[start..].find('>').unwrap() + start + 1..end];
        let data: serde_json::Value =
            serde_json::from_str(json_text).expect("embedded JSON parses");
        assert_eq!(data["property"], "grid::branchy");
        let property_fn = data["property_fn"].as_str().unwrap_or_default();
        assert!(
            property_fn
                .starts_with("ephyra::runner::tests::the_coverage_grid_is_written::{closure#"),
            "{property_fn}"
        );
        assert!(
            data["files"].as_array().unwrap().iter().any(|f| {
                f["functions"].as_array().unwrap().iter().any(|func| {
                    func["name"] == "ephyra::runner::tests::branchy"
                        && func["lines"].as_array().unwrap().iter().any(|l| l[1] == 0)
                        && func["lines"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|l| l[1].as_u64().unwrap() > 0)
                })
            }),
            "{data}"
        );
        assert_eq!(data["targets"][0]["hits"], 0);
    }

    #[cfg(feature = "coverage")]
    #[test]
    fn coverage_summary_matches_the_build() {
        let report = check(&cfg(), gens::vec(gens::int(0..=9)), |v: Vec<i32>| v.len() <= 100);
        if crate::coverage::available() {
            let coverage = report.coverage.as_ref().expect("coverage collected");
            assert!(coverage.functions_total > 0);
            assert_eq!(coverage.inputs, 64);
            assert!(report.to_string().contains("coverage:"), "{report}");
        } else {
            assert!(report.coverage.is_none());
            assert!(
                !report.health.iter().any(|w| w.kind.starts_with("coverage")),
                "{:?}",
                report.health
            );
        }
        let mut wants = cfg();
        wants.coverage.guided = true;
        let report = check(&wants, gens::vec(gens::int(0..=9)), |v: Vec<i32>| v.len() <= 100);
        if crate::coverage::available() {
            assert!(report.target.is_some(), "guided search seeds the pool from novelty: {report}");
        } else {
            assert!(
                report.health.iter().any(|w| w.kind == "coverage_unavailable"),
                "{:?}",
                report.health
            );
        }
    }

    #[test]
    fn hand_written_loops_replay_by_seed() {
        fn climb(seed: u64) -> Report {
            let mut config = cfg();
            config.seed = Some(seed);
            config.tests = 100;
            config.max_size = 20;
            let mut run = Run::new(&config, gens::vec(gens::int(0i32..=9)), |v: Vec<i32>| {
                let sum: i32 = v.iter().sum();
                crate::target(f64::from(sum));
                sum < 500
            });
            while run.wants_more() {
                let Some(input) = run.draw() else { continue };
                let tested = run.test(&input);
                if tested.passed() {
                    run.keep(input.value, &tested);
                }
            }
            for _ in 0..200 {
                if run.report().status != Status::Passed {
                    break;
                }
                let Some(parent) = run.pool_pick() else { break };
                let Some(child) = run.mutate(&parent) else { continue };
                let tested = run.test(&child);
                if tested.failed() {
                    break;
                }
                run.keep(child.value, &tested);
            }
            run.finish()
        }
        let a = climb(11);
        let b = climb(11);
        assert_eq!(a.status, Status::Passed, "{a}");
        assert_eq!((a.tests, a.iterations), (b.tests, b.iterations));
        assert_eq!(a.iterations, 300, "100 draws and 200 mutations: {a}");
        assert_eq!(a.tests + a.discarded, a.iterations, "passing mutants count as tests");
        let target = a.target.as_ref().expect("mutations are summarised");
        assert_eq!(target.iterations, 200);
        assert_eq!(target.best_score, b.target.as_ref().unwrap().best_score);
        assert_ne!(climb(12).seed, a.seed);
    }

    #[test]
    fn a_failure_found_by_mutation_is_reported_without_a_replay_point() {
        let mut config = cfg();
        config.seed = Some(5);
        config.max_size = 8;
        let mut run = Run::new(&config, gens::vec(gens::int(0i32..=9)), |v: Vec<i32>| v.is_empty());
        let input = loop {
            if let Some(input) = run.draw()
                && !input.value.is_empty()
            {
                break input;
            }
        };
        assert!(input.replay().is_some(), "drawn inputs regenerate from a point");
        let mutant = loop {
            if let Some(mutant) = run.mutate(&input.value)
                && !mutant.value.is_empty()
            {
                break mutant;
            }
        };
        assert!(mutant.replay().is_none(), "mutants do not");
        assert_eq!(mutant.origin(), crate::report::Origin::Mutation);
        let tested = run.test(&mutant);
        assert!(tested.failed());
        assert!(!run.wants_more(), "a failure stops the loop");
        let report = run.finish();
        assert_eq!(report.status, Status::Failed, "{report}");
        let failure = report.failure.expect("failure");
        assert!(failure.replay.is_none(), "found by mutation: {failure:?}");
        assert_eq!(failure.iteration, mutant.iteration());
        assert_eq!(report.tests, 0, "nothing passed");
    }

    #[test]
    fn loops_can_shrink_on_their_own_terms() {
        fn failing_run() -> Run<impl Gen<Value = Vec<i32>>, impl FnMut(Vec<i32>) -> bool> {
            let mut config = cfg();
            config.seed = Some(3);
            config.max_size = 30;
            let mut run =
                Run::new(&config, gens::vec(gens::int(0i32..=9)).len(..=30), |v: Vec<i32>| {
                    v.iter().sum::<i32>() < 20
                });
            while run.wants_more() {
                let Some(input) = run.draw() else { continue };
                run.test(&input);
            }
            assert!(run.failure().is_some(), "{}", run.report());
            run
        }

        // The built-in driver, on demand.
        let mut run = failing_run();
        let original = run.failure().unwrap().clone();
        assert!(run.reproduces(&original), "the original reproduces");
        assert!(!run.reproduces(&vec![]), "an empty list passes");
        let minimal = run.shrink().unwrap().clone();
        let sum: i32 = minimal.iter().sum();
        assert!(
            sum >= 20 && sum <= original.iter().sum() && minimal.len() <= original.len(),
            "{minimal:?}"
        );
        let report = run.finish();
        let failure = report.failure.expect("failed");
        assert_eq!(failure.shrunk.as_ref().map(|c| c.debug.clone()), Some(format!("{minimal:?}")));
        assert!(failure.shrink.unwrap().steps >= 3, "the loop's steps count too");

        // A greedy shrinker written by hand: drop one element at a time.
        let mut run = failing_run();
        let mut current = run.failure().unwrap().clone();
        'outer: loop {
            for i in 0..current.len() {
                let mut candidate = current.clone();
                candidate.remove(i);
                if run.reproduces(&candidate) {
                    current = candidate;
                    continue 'outer;
                }
            }
            break;
        }
        assert!(run.shrunk_to(current.clone()));
        let report = run.finish();
        let failure = report.failure.as_ref().expect("failed");
        assert_eq!(failure.shrunk.as_ref().map(|c| c.debug.clone()), Some(format!("{current:?}")));
        assert!(
            current.iter().sum::<i32>() >= 20 && current.iter().sum::<i32>() < 30,
            "{current:?}"
        );
        assert!(!report.health.iter().any(|w| w.kind == "shrunk_to"), "{report}");

        // A wrong claim is refused.
        let mut run = failing_run();
        assert!(!run.shrunk_to(vec![1]));
        let report = run.finish();
        assert!(report.health.iter().any(|w| w.kind == "shrunk_to"), "{report}");
        assert!(report.failure.unwrap().shrunk.is_none(), "the original stands");
    }

    #[test]
    fn size_increases_logarithmically_and_respects_the_maximum() {
        let sizes: Vec<usize> = (0..16).map(|i| size_for(i, 100)).collect();
        assert_eq!(sizes, [0, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3, 4]);
        assert_eq!(size_for(1_023, 100), 10);
        assert_eq!(size_for(u64::MAX, 100), 63);
        assert_eq!(size_for(u64::MAX, 7), 7);
        assert_eq!(size_for(17, 0), 0);
    }

    #[cfg(feature = "json")]
    #[test]
    fn report_serialises_to_json() {
        let report = check(&cfg(), gens::int(0..=9), |x: i32| x < 5);
        let json = report.to_json();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["status"], "Failed");
        assert_eq!(value["seed"], 1);
        assert_eq!(value["failure"]["shrunk"]["debug"], "5");
        assert_eq!(value["failure"]["replay"]["seed"], 1);
        assert!(value["failure"]["shrink"]["steps"].is_number());
    }
}
