// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Running a test case: generating an input, running the property on it
//! with panic capture, coverage and the sinks, and what the run showed.

use std::{
    fmt::{
        self,
        Debug,
        Write as _,
    },
    hash::{
        DefaultHasher,
        Hasher,
    },
    time::Instant,
};

use {
    super::{
        Outcome,
        Run,
        Testable,
        coverage::CoverageState,
        labels,
        panic::{
            self,
            PanicInfo,
        },
        shrinking::Found,
        stats::{
            Provenance,
            cap,
            micros,
        },
    },
    crate::{
        Gen,
        Source,
        config::ReplayPoint,
        report::{
            Abort,
            FailureKind,
            Status,
        },
        source::{
            mix,
            size_for,
        },
    },
};

/// The property, with what is collected around each run of it: coverage
/// and the sinks.
pub(super) struct Exec<P> {
    pub(super) property: P,
    pub(super) coverage: CoverageState,
    /// Where each evaluated input goes: the observation file, when
    /// `Config::report.observations` is set, and the installed sinks.
    pub(super) sinks: Vec<Box<dyn crate::sink::RunSink>>,
}

/// One run of the property.
pub(super) struct Ran {
    pub(super) result: Result<Outcome, PanicInfo>,
    pub(super) observed: labels::Observed,
    pub(super) elapsed: std::time::Duration,
    /// See [`Tested::novelty`].
    pub(super) novelty: u32,
}

impl<P> Exec<P> {
    /// Whether any sink is open, so inputs are rendered and observed.
    pub(super) fn observing(&self) -> bool {
        !self.sinks.is_empty()
    }

    /// Run the property on `value` with panic capture and coverage, and pass
    /// it to the sinks. `shrinking` is the failure a shrink candidate is
    /// compared with.
    pub(super) fn run<T: Debug, R: Testable>(
        &mut self,
        value: T,
        at: At,
        shrinking: Option<&Signature>,
    ) -> Ran
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

    pub(super) fn emit(&mut self, case: &crate::sink::TestCase<'_>) {
        for sink in &mut self.sinks {
            sink.test_case(case);
        }
    }
}

/// An input's status and reason, as sinks receive them.
pub(super) fn case_status(
    result: &Result<Outcome, PanicInfo>,
) -> (crate::sink::CaseStatus, String) {
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
pub(super) struct At {
    pub(super) origin: &'static str,
    pub(super) seed: u64,
    pub(super) iteration: u64,
    pub(super) size: usize,
    /// How long generating it took.
    pub(super) generate: std::time::Duration,
}

/// Why an input could not be generated.
pub(super) enum Skipped {
    /// The generator discarded it, after the given time.
    Discarded(String, std::time::Duration),
    Panicked(PanicInfo),
}

/// An input drawn or mutated by a [`Run`], ready to be tested.
#[derive(Clone, Debug)]
pub struct Input<T> {
    /// The value.
    pub value: T,
    pub(super) point: Option<ReplayPoint>,
    pub(super) size: usize,
    pub(super) iteration: u64,
    pub(super) origin: crate::report::Origin,
    pub(super) generate: std::time::Duration,
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
    pub(super) fn at(&self, origin: &'static str, seed: u64) -> At {
        let seed = self.point.map_or(seed, |p| p.seed);
        At { origin, seed, iteration: self.iteration, size: self.size, generate: self.generate }
    }
}

/// The three ways testing one input can end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Verdict {
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
    pub(super) verdict: Verdict,
    pub(super) novelty: u32,
    pub(super) score: Option<f64>,
    pub(super) elapsed: std::time::Duration,
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
    /// Run `make` (the generator's `generate` or `mutate`) for `iteration`
    /// with randomness from `seed`, counting its time as generation.
    pub(super) fn produce(
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
    pub(super) fn generate(
        &mut self,
        point: ReplayPoint,
        origin: crate::report::Origin,
    ) -> Result<Input<G::Value>, Skipped> {
        let input =
            self.produce(point.seed, point.iteration, origin, |g, source| g.generate(source))?;
        Ok(Input { point: Some(point), ..input })
    }

    /// The value at `point`, if it generates without discard or panic.
    pub(super) fn regenerate(&mut self, point: ReplayPoint) -> Option<G::Value> {
        self.generate(point, crate::report::Origin::Replay).ok().map(|input| input.value)
    }

    /// Run the property on `value` (see [`Exec::run`]), counting its time.
    pub(super) fn run_property(&mut self, value: G::Value, at: At) -> Ran {
        let ran = self.exec.run(value, at, None);
        self.report.prop_time += ran.elapsed;
        self.stats.prop_micros.push(micros(ran.elapsed));
        ran
    }

    /// Count a generated input, and the growth of the distinct set.
    pub(super) fn note_distinct(&mut self, hash: u64) {
        if self.seen.insert(hash) {
            self.stats.distinct_growth.push(self.generated, self.seen.len() as u64);
        }
    }

    pub(super) fn next_iteration(&mut self) -> u64 {
        let iteration = self.report.iterations;
        self.report.iterations += 1;
        iteration
    }

    /// The observation line of an input the generator discarded after
    /// `generate`.
    pub(super) fn observe_generation_discard(
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
    pub(super) fn abort(&mut self, info: PanicInfo, iteration: u64) {
        self.report.status = Status::Aborted;
        self.report.abort =
            Some(Abort { message: info.message, location: info.location, iteration });
    }

    /// Draw the input at `point`. A discard by the generator is counted
    /// and a generator panic aborts the run; both yield `None`.
    pub(super) fn draw_at(
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
    pub(super) fn test_input(
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

    /// A variant of `parent` from the generator's mutator, at this
    /// iteration's size. The mutator itself returns a value;
    /// this runner wrapper returns `None` if it discarded or panicked
    /// (a panic aborts the run).
    pub fn mutate(&mut self, parent: &G::Value) -> Option<Input<G::Value>> {
        let iteration = self.next_iteration();
        self.mutations += 1;
        self.mutate_at(parent, iteration)
    }

    pub(super) fn mutate_at(
        &mut self,
        parent: &G::Value,
        iteration: u64,
    ) -> Option<Input<G::Value>> {
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
}

/// Identifies "the same failure" while shrinking: the kind, and for panics
/// the site. Messages are ignored because they usually embed the input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Signature {
    pub(super) kind: FailureKind,
    pub(super) panic_site: Option<(String, u32)>,
}

impl Signature {
    pub(super) fn of(result: &Result<Outcome, PanicInfo>) -> Option<Self> {
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

/// Run the property once with panic capture and the side channel active.
pub(crate) fn evaluate<T, R: Testable, P: FnMut(T) -> R>(
    property: &mut P,
    value: T,
    trace_observations: bool,
) -> (Result<Outcome, PanicInfo>, labels::Observed) {
    labels::collect(trace_observations, || panic::catch(|| property(value).into_outcome()))
}

/// Hash a value's `Debug` rendering without allocating it.
pub(super) fn debug_hash<T: Debug>(value: &T) -> u64 {
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
