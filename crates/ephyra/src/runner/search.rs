// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The search: the stopping rules, the random phase, and the targeted
//! phase over the pool of kept inputs.

use std::fmt::Debug;

use {
    super::{
        Run,
        Testable,
        exec::Tested,
        stats::cap,
    },
    crate::{
        Gen,
        config::ReplayPoint,
        report::{
            Status,
            TargetSummary,
        },
        source::u01,
    },
};

impl<G, R, P> Run<G, P>
where
    G: Gen,
    G::Value: Clone + Debug,
    R: Testable,
    P: FnMut(G::Value) -> R,
{
    /// Whether the run still wants inputs: none of the stopping rules has
    /// fired. The rules, in order: the number of tests reached
    /// [`Config::tests`](crate::Config::tests); the time budget
    /// ([`Config::budget_ms`](crate::Config::budget_ms)) ran out; the timeout
    /// ([`Config::timeout_ms`](crate::Config::timeout_ms)) elapsed; more
    /// inputs were discarded than
    /// [`Config::max_discard_ratio`](crate::Config::max_discard_ratio)
    /// allows; the property failed, or the generator panicked. A rule that
    /// fired is reflected in the report.
    ///
    /// With [`Config::failures`](crate::Config::failures) above 1, a
    /// failure kept by [`test`](Self::test) is first shrunk, remembered and
    /// recorded, and the run stops only once that many distinct failures
    /// are found.
    pub fn wants_more(&mut self) -> bool {
        if !self.carry_on() || self.report.status != Status::Passed {
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
    pub(super) fn timed_out_or_gave_up(&mut self, discarded: u64, tests: u64) -> bool {
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
    pub(super) fn target_summary(&self, iterations: u64) -> Option<TargetSummary> {
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
    pub(super) fn random_search(&mut self) {
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

    /// The targeted phase: mutate the best-scoring inputs to push the score
    /// up, looking for a failure along the way.
    pub(super) fn targeted_search(&mut self) {
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
                if self.carry_on() {
                    continue;
                }
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
}
