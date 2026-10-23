// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Remembered inputs: the replay point and the regressions file.

use std::fmt::Debug;

use {
    super::{
        Run,
        Testable,
        exec::Skipped,
        shrinking::Found,
    },
    crate::{
        Gen,
        report::ReplayOutcome,
    },
};
#[cfg(feature = "json")]
use {
    super::{
        exec::Signature,
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
}
