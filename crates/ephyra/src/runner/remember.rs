// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Remembered inputs: the replay point and the regressions file.

use std::fmt::Debug;

use {
    super::{
        Run,
        Testable,
        exec::Skipped,
    },
    crate::{
        Gen,
        report::ReplayOutcome,
    },
};
#[cfg(feature = "json")]
use {
    super::{
        shrinking::Found,
        stats::cap,
    },
    crate::{
        config::ReplayPoint,
        report::Counterexample,
    },
};

impl<G, R, P> Run<G, P>
where
    G: Gen,
    G::Value: Clone + Debug,
    R: Testable,
    P: FnMut(G::Value) -> R,
{
    /// Test the remembered inputs first: the replay point
    /// ([`Config::replay`](crate::Config::replay), `EPHYRA_REPLAY`) and the
    /// property's stored regressions. Neither counts as a test. Returns
    /// whether one failed, in which case the loop should stop and
    /// [`finish`](Self::finish) reports it.
    ///
    /// The first [`wants_more`](Self::wants_more) and the first iteration of
    /// a shipped search [loop](crate::loops) call this, unless the run has
    /// ended before. A loop calls it itself only when it uses neither, or to
    /// do something between the replay and the search. A run tests its
    /// remembered inputs once: a later call returns the same answer.
    ///
    /// With [`Config::failures`](crate::Config::failures) above 1, every
    /// stored regression is replayed and each failing one is kept like a
    /// failure [`test`](Self::test) found; `true` then means the replay
    /// point failed or that many distinct failures are already known, and
    /// on `false` the search goes on for the rest.
    pub fn remembered(&mut self) -> bool {
        if let Some(failed) = self.remembered {
            return failed;
        }
        let failed = self.test_remembered();
        self.remembered = Some(failed);
        failed
    }

    /// Warn about remembered inputs when no step of the loop tested them.
    pub(super) fn warn_untested(&mut self) {
        if self.remembered.is_some() {
            return;
        }
        const WHY: &str = "the run ended before a call to `wants_more` or `remembered`";
        if let Some(point) = self.config.replay {
            self.report.warn("replay", format!("replay point {point} was not tested: {WHY}"));
        }
        #[cfg(feature = "json")]
        if self.config.regressions.replay {
            let dir = &self.config.regressions.dir;
            let stored = self.stored().len();
            if stored > 0 {
                let file = crate::regressions::path_for(dir, &self.report.name);
                self.report.warn(
                    "regressions",
                    format!(
                        "{stored} input(s) stored in {} were not tested: {WHY}",
                        file.display()
                    ),
                );
            }
        }
    }

    /// The stored regressions, without those that this run appended.
    #[cfg(feature = "json")]
    fn stored(&self) -> Vec<crate::regressions::Entry> {
        let mut entries = crate::regressions::load(&self.config.regressions.dir, &self.report.name);
        entries.truncate(entries.len().saturating_sub(self.appended));
        entries
    }

    /// [`remembered`](Self::remembered), the first time.
    fn test_remembered(&mut self) -> bool {
        if let Some(point) = self.config.replay {
            let origin = crate::report::Origin::Replay;
            let failed = match self.generate(point, origin) {
                Ok(input) => self.test_input(&input, false).1,
                Err(Skipped::Discarded(reason, generate)) => {
                    self.observe_generation_discard(point, origin, &reason, generate);
                    None
                },
                Err(Skipped::Panicked(_)) => None,
            };
            self.report.replay = Some(ReplayOutcome { point, reproduced: failed.is_some() });
            if let Some(found) = failed {
                self.note_failure(found);
                return true;
            }
        }

        #[cfg(feature = "json")]
        if self.config.regressions.replay {
            let entries = self.stored();
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
                let ran = self.run_property(input.value.clone(), at);
                if let Some(signature) = ran.signature {
                    self.note_failure(Found { input, result: ran.result, signature });
                    if !self.carry_on() {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Remember the failure at `point` in the regressions file, with its
    /// renderings.
    #[cfg(feature = "json")]
    pub(super) fn remember_failure(
        &mut self,
        point: ReplayPoint,
        original: &Counterexample,
        shrunk: Option<&Counterexample>,
    ) {
        let entry = crate::regressions::Entry::new(
            &self.report.name,
            point.seed,
            point.iteration,
            original.debug.clone(),
            shrunk.map(|c| c.debug.clone()),
        );
        match crate::regressions::save(&self.config.regressions.dir, &entry) {
            Ok((path, appended)) => {
                self.report.regressions.file = Some(path);
                self.report.regressions.saved |= appended;
                self.appended += usize::from(appended);
            },
            Err(e) => {
                self.report.warn(
                    "regressions",
                    format!("could not write {}: {e}", self.config.regressions.dir.display()),
                )
            },
        }
    }
}
