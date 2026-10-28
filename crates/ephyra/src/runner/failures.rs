// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Several distinct failures in one run (`Config::failures` above 1): which
//! failures are new, when the run goes on, and the report's list.

use std::fmt::Debug;

use {
    super::{
        Run,
        Testable,
        shrinking::Found,
    },
    crate::{
        Gen,
        report::{
            DistinctFailure,
            Status,
        },
    },
};

impl<G, R, P> Run<G, P>
where
    G: Gen,
    G::Value: Clone + Debug,
    R: Testable,
    P: FnMut(G::Value) -> R,
{
    /// How many distinct failures the run looks for.
    pub(super) fn wanted(&self) -> usize {
        self.config.failures.max(1)
    }

    /// A failure the run came across: kept for shrinking when it is the
    /// first, or, when the run looks for several, the first with its
    /// signature; otherwise only counted.
    pub(super) fn note_failure(&mut self, found: Found<G::Value>) {
        if self.wanted() == 1 {
            if self.pending.is_none() {
                self.pending = Some(found);
            }
            return;
        }
        if let Some((_, count)) = self.distinct.iter_mut().find(|(s, _)| *s == found.signature) {
            *count += 1;
            return;
        }
        // A loop that tests on without asking `wants_more` moves on from the
        // failure it kept.
        if self.pending.is_some() {
            self.settle_distinct();
        }
        if self.distinct.len() < self.wanted() {
            self.distinct.push((found.signature.clone(), 1));
            self.pending = Some(found);
        }
    }

    /// Whether the run should go on after its failures so far. A run that
    /// stops at the first failure goes on while there is none; one that
    /// looks for several first records the failure it kept, then goes on
    /// until it has found that many.
    pub(crate) fn carry_on(&mut self) -> bool {
        if self.wanted() == 1 {
            return self.pending.is_none();
        }
        if self.pending.is_some() {
            self.settle_distinct();
        }
        self.distinct.len() < self.wanted()
    }

    /// Shrink, localize and remember the kept failure and add it to the
    /// report's list; the first one is also the report's failure.
    fn settle_distinct(&mut self) {
        let index = self.report.failures.len();
        let Some(failure) = self.settle() else {
            return;
        };
        let signature = self.distinct.get(index).map(|(s, _)| s.name()).unwrap_or_default();
        if self.report.failure.is_none() {
            self.report.failure = Some(failure.clone());
        }
        self.report.failures.push(DistinctFailure { signature, count: 0, failure });
    }

    /// At the end of a run that looked for several failures: record the
    /// one still kept, the counts, and the status.
    pub(super) fn finish_failures(&mut self) {
        if self.pending.is_some() {
            self.settle_distinct();
        }
        for (found, (_, count)) in self.report.failures.iter_mut().zip(&self.distinct) {
            found.count = *count;
        }
        if self.report.failures.is_empty() {
            return;
        }
        if let Some(abort) = &self.report.abort {
            let message = format!(
                "the generator panicked at iteration {} after the failures below were found: {}",
                abort.iteration, abort.message
            );
            self.report.warn("aborted", message);
        }
        self.report.status = Status::Failed;
    }
}
