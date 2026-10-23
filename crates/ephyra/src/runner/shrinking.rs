// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Shrinking the pending failure, by the built-in driver or by the loop,
//! and turning it into the report's failure.

use std::{
    fmt::Debug,
    time::Instant,
};

use {
    super::{
        Outcome,
        Run,
        Testable,
        exec::{
            At,
            Input,
            Signature,
            evaluate,
        },
        panic::PanicInfo,
        stats::{
            cap,
            micros,
        },
    },
    crate::{
        Gen,
        report::{
            Counterexample,
            Failure,
            FailureKind,
            Location,
            ShrinkStats,
            Status,
        },
        shrink,
    },
};

/// A failing input, kept for [`Run::finish`].
pub(super) struct Found<T> {
    pub(super) input: Input<T>,
    pub(super) result: Result<Outcome, PanicInfo>,
}

impl<T> Found<T> {
    /// Where inputs derived from the failure (shrink candidates, neighbours,
    /// the minimal input) sit in the observation file.
    pub(super) fn derived(&self, origin: &'static str, seed: u64) -> At {
        let (iteration, size) = (self.input.iteration, self.input.size);
        At { origin, seed, iteration, size, generate: std::time::Duration::ZERO }
    }
}

/// The pending failure's shrinking: what it fails like, the smallest input
/// known to fail the same way, and the steps taken so far.
pub(super) struct Shrinking<T> {
    pub(super) signature: Signature,
    /// The failure did not reproduce when rerun; nothing is shrunk.
    pub(super) flaky: bool,
    pub(super) minimal: T,
    /// The message and location of the failure `minimal` produced, once a
    /// candidate was accepted.
    pub(super) failure: shrink::SameFailure,
    pub(super) stats: ShrinkStats,
    /// The built-in driver ran, or the loop supplied its own result.
    pub(super) done: bool,
    pub(super) started: Option<Instant>,
}

impl<G, R, P> Run<G, P>
where
    G: Gen,
    G::Value: Clone + Debug,
    R: Testable,
    P: FnMut(G::Value) -> R,
{
    /// Rerun the pending failure once to see that it reproduces (the
    /// flakiness check) and set up shrinking. Returns whether there is a
    /// failure to shrink.
    pub(super) fn reproduce_pending(&mut self) -> bool {
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
    /// Each call is a shrinking step: beyond
    /// [`Config::shrink`](crate::Config::shrink)'s budget the answer is
    /// always `false` and the report says the budget ran out. `false`
    /// without a failure, or when it did not reproduce.
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
    pub(super) fn shrink_step(
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
    /// the generator's candidates, within
    /// [`Config::shrink`](crate::Config::shrink)'s budget. Returns the
    /// smallest input that fails the same way; `None` without a failure, or
    /// when it did not reproduce. [`finish`](Self::finish) then reports this
    /// result and shrinks no further.
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
    pub(super) fn drive_shrink(&mut self, shrinking: &mut Shrinking<G::Value>, at: At) {
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

    /// Handle the pending failure: check it reproduces, shrink it (unless
    /// the loop already did), remember it, and fill in the report.
    pub(super) fn fail(&mut self) {
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
            self.localize(&minimal, input.size, at, location.as_ref());
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
            self.remember_failure(point, &original, shrunk.as_ref());
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
}

/// Kind, message and location of a failing result.
pub(super) fn details(
    result: Result<Outcome, PanicInfo>,
) -> (FailureKind, Option<String>, Option<Location>) {
    match result {
        Ok(Outcome::Fail(message)) => (FailureKind::Error, Some(message), None),
        Err(info) => (FailureKind::Panic, Some(info.message), info.location),
        Ok(_) => (FailureKind::False, None, None),
    }
}

/// The message and location of `result` when it fails like `signature`.
pub(super) fn same_failure(
    result: Result<Outcome, PanicInfo>,
    signature: &Signature,
) -> shrink::SameFailure {
    if Signature::of(&result).as_ref() != Some(signature) {
        return None;
    }
    let (_, message, location) = details(result);
    Some((message, location))
}

/// `value`'s `Debug` rendering, cut at `limit` characters.
pub(super) fn counterexample<T: Debug>(value: &T, limit: usize) -> Counterexample {
    let debug = format!("{value:?}");
    Counterexample { truncated: debug.chars().nth(limit).is_some(), debug: cap(debug, limit) }
}
