// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Remembered inputs: the replay point and the regressions file.

use std::fmt::Debug;

use {
    super::{
        Run,
        Testable,
        exec::{
            Input,
            Skipped,
            debug_hash,
        },
    },
    crate::{
        Gen,
        config::ReplayPoint,
        report::ReplayOutcome,
    },
};
#[cfg(feature = "json")]
use {
    super::{
        exec::Signature,
        shrinking::Found,
        stats::{
            SAMPLE_CHARS,
            cap,
        },
    },
    crate::report::{
        Counterexample,
        Origin,
    },
};

/// How an input was made, so that it can be made again: drawn at a point, or
/// mutated from `parent`, a value or the hash of a seed, at `iteration`.
#[derive(Clone, Copy, Debug)]
pub(super) enum Made<P> {
    Drawn(ReplayPoint),
    Mutated { parent: P, iteration: u64 },
}

impl<T: Clone> Input<T> {
    /// How it was made, if that is known.
    pub(super) fn made(&self) -> Option<Made<T>> {
        match (self.point, self.parent.clone()) {
            (Some(point), _) => Some(Made::Drawn(point)),
            (None, Some(parent)) => Some(Made::Mutated { parent, iteration: self.iteration }),
            (None, None) => None,
        }
    }
}

/// How often an input that does not render as stored is regenerated. A hash
/// collection of two items then shows only one of its two orders once in
/// 65,536 times.
#[cfg(feature = "json")]
const REGENERATIONS: usize = 16;

/// The value that renders as `stored`, a rendering cut at `limit` characters:
/// `first`, or one of those that `again` generates from the same point. Hash
/// collections render in another order every time, so when no rendering is
/// the stored one but they differ among themselves, `first` will do if it has
/// the same words in another order. The flag is whether it renders as stored.
#[cfg(feature = "json")]
fn as_stored<T: Debug>(
    stored: &str,
    limit: usize,
    first: T,
    mut again: impl FnMut() -> Option<T>,
) -> Option<(T, bool)> {
    let debug = format!("{first:?}");
    if cap(debug.clone(), limit) == stored {
        return Some((first, true));
    }
    let mut unordered = false;
    for _ in 0..REGENERATIONS {
        let value = again()?;
        let text = format!("{value:?}");
        unordered |= text != debug;
        if cap(text, limit) == stored {
            return Some((value, true));
        }
    }
    (unordered && reordered(stored, &debug)).then_some((first, false))
}

/// Whether the words of `stored`, and the marks between them, are those of
/// `debug` in another order. A cut rendering shows only some of them: it is
/// better to replay an input that may have changed than to skip one that did
/// not.
#[cfg(feature = "json")]
fn reordered(stored: &str, debug: &str) -> bool {
    let mark = |c: char| !c.is_alphanumeric();
    let words = |text| str::split(text, mark).chain(str::matches(text, mark)).collect::<Vec<_>>();
    let (shown, cut) = match stored.strip_suffix('…') {
        Some(shown) => (shown, true),
        None => (stored, false),
    };
    // A cut word may go on behind the cut.
    let shown = if cut { shown.trim_end_matches(char::is_alphanumeric) } else { shown };
    let (mut some, mut all) = (words(shown), words(debug));
    some.sort_unstable();
    all.sort_unstable();
    let mut rest = all.iter();
    (cut || some.len() == all.len()) && some.iter().all(|word| rest.any(|other| other == word))
}

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
    /// With [`Config::failures`](crate::Config::failures) above 1, every
    /// stored regression is replayed and each failing one is kept like a
    /// failure [`test`](Self::test) found; `true` then means the replay
    /// point failed or that many distinct failures are already known, and
    /// on `false` the search goes on for the rest.
    pub fn remembered(&mut self) -> bool {
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
                let stored = match entry.mutations.len() {
                    0 => format!("stored input {point}"),
                    n => format!("stored input {point} (mutations: {n})"),
                };
                self.report.regressions.replayed += 1;
                let Ok(mut input) = self.remake(point, &entry.mutations) else {
                    self.report.regressions.stale += 1;
                    self.report.warn("regressions", format!("{stored} could not be regenerated (discarded or the generator panicked)"));
                    continue;
                };
                let debug = cap(format!("{:?}", input.value), limit);
                let again = || Some(self.remake(point, &entry.mutations).ok()?.value);
                let found = as_stored(&entry.debug, limit, input.value, again);
                let Some((value, same_order)) = found else {
                    self.report.regressions.stale += 1;
                    self.report.warn(
                        "regressions",
                        format!(
                            "{stored} no longer matches the generator (stored {}, regenerated {debug}); \
                             delete the entry or rerun the property",
                            entry.debug
                        ),
                    );
                    continue;
                };
                input.value = value;
                let at = input.at("regression");
                let ran = self.run_property(input.value.clone(), at);
                if let Some(signature) = ran.signature {
                    self.note_failure(Found { input, result: ran.result, signature });
                    if !self.carry_on() {
                        return true;
                    }
                } else if !same_order {
                    self.report.warn(
                        "regressions",
                        format!(
                            "{stored} did not fail, but its items were not in the \
                             stored order, which may be part of the failure"
                        ),
                    );
                }
            }
        }
        false
    }

    /// Whether failing inputs are saved.
    pub(super) fn saves(&self) -> bool {
        cfg!(feature = "json") && self.config.regressions.save
    }

    /// `value` becomes a seed for mutation. When it is the last input tested,
    /// note how it was made, so that a failing mutant of it can be remembered.
    pub(crate) fn note_seed(&mut self, value: &G::Value) {
        if self.saves()
            && let Some((hash, made)) = &self.tested
            && debug_hash(value) == *hash
            && let Some(made) = self.known(made)
        {
            // The first way to make a value stays, so that no seed descends
            // from itself.
            self.seeds.entry(*hash).or_insert(made);
        }
    }

    /// `made` in terms of this run's seeds, when it can be.
    fn known(&self, made: &Made<G::Value>) -> Option<Made<u64>> {
        match made {
            Made::Drawn(point) => (point.seed == self.report.seed).then_some(Made::Drawn(*point)),
            Made::Mutated { parent, iteration } => {
                let (parent, iteration) = (debug_hash(parent), *iteration);
                let noted = self.seeds.contains_key(&parent);
                noted.then_some(Made::Mutated { parent, iteration })
            },
        }
    }

    /// The input generated at `point` and then mutated at each of `mutations`.
    #[cfg(feature = "json")]
    fn remake(
        &mut self,
        point: ReplayPoint,
        mutations: &[u64],
    ) -> Result<Input<G::Value>, Skipped> {
        let mut input = self.generate(point, Origin::Replay)?;
        for &iteration in mutations {
            let parent = input.value;
            input = self.produce(point.seed, iteration, Origin::Replay, |g, source| {
                g.mutate(&parent, source)
            })?;
        }
        Ok(input)
    }

    /// The point and the mutations that [`remake`](Self::remake) a failing
    /// `input`; for a mutant, after they did and it failed like `signature`.
    #[cfg(feature = "json")]
    fn recipe(
        &mut self,
        input: &Input<G::Value>,
        signature: &Signature,
    ) -> Option<(ReplayPoint, Vec<u64>)> {
        if let Some(point) = input.point {
            return Some((point, Vec::new()));
        }
        let mut made = self.known(&input.made()?)?;
        let mut mutations = Vec::new();
        let point = loop {
            match made {
                Made::Drawn(point) => break point,
                Made::Mutated { parent, iteration } => {
                    mutations.push(iteration);
                    made = *self.seeds.get(&parent)?;
                },
            }
        };
        mutations.reverse();
        // Each time: a mutant that depends on the order of a hash collection
        // is made again only by chance.
        let text = format!("{:?}", input.value);
        let mut value = None;
        for _ in 0..REGENERATIONS {
            let made = self.remake(point, &mutations).ok()?.value;
            let debug = format!("{made:?}");
            if debug == text || value.is_none() {
                value = Some(made);
            }
            if debug != text && !reordered(&text, &debug) {
                return None;
            }
        }
        let value = value?;
        (self.rerun(&value).as_ref() == Some(signature)).then_some((point, mutations))
    }

    /// Remember the `input` that failed like `signature` in the regressions
    /// file, with its renderings.
    #[cfg(feature = "json")]
    pub(super) fn remember_failure(
        &mut self,
        input: &Input<G::Value>,
        signature: &Signature,
        original: &Counterexample,
        shrunk: Option<&Counterexample>,
    ) {
        if input.point.is_none() && input.origin == Origin::Replay {
            return; // A mutant from the file.
        }
        let Some((point, mutations)) = self.recipe(input, signature) else {
            self.report.warn(
                "regressions",
                format!(
                    "the failing input {} was found by mutation and could not be saved, because \
                     it cannot be made again on its own; repeat the search with EPHYRA_SEED={}",
                    cap(original.debug.clone(), SAMPLE_CHARS),
                    self.report.seed
                ),
            );
            return;
        };
        let entry = crate::regressions::Entry::new(
            &self.report.name,
            point,
            mutations,
            original.debug.clone(),
            shrunk.map(|c| c.debug.clone()),
        );
        match crate::regressions::save(&self.config.regressions.dir, &entry) {
            Ok((path, appended)) => {
                self.report.regressions.file = Some(path);
                self.report.regressions.saved |= appended;
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

#[cfg(all(test, feature = "json"))]
mod tests {
    use super::*;

    /// Renders as its text.
    struct Text(&'static str);

    impl Debug for Text {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.0)
        }
    }

    #[test]
    fn an_input_is_the_stored_one_when_a_rendering_or_its_words_match() {
        let found = |stored: &str, renderings: &'static [&'static str]| {
            let mut texts = renderings.iter().cycle().map(|text| Text(text));
            let first = texts.next().unwrap();
            as_stored(stored, 8, first, || texts.next()).map(|(text, same)| (text.0, same))
        };
        assert_eq!(found("[1]", &["[1]"]), Some(("[1]", true)));
        assert_eq!(found("[1]", &["[2]"]), None, "a stable rendering must match");
        assert_eq!(found("[1, 2]", &["[2, 1]"]), None, "the order of a stable one counts");
        assert_eq!(found("{1, 2}", &["{2, 1}", "{1, 2}"]), Some(("{1, 2}", true)), "the match");
        let three = &["{3, 2, 1}", "{3, 1, 2}"];
        assert_eq!(found("{1, 2, 3}", three), Some(("{3, 2, 1}", false)), "the same items");
        assert_eq!(found("{1, 2, 4}", three), None, "another item");
        assert_eq!(found("{1, 2}", three), None, "an item more");
        assert_eq!(found("{12, 34}", &["{21, 43}", "{43, 21}"]), None, "the same digits");
        assert_eq!(found("{'+', '-'}", &["{'<', '>'}", "{'>', '<'}"]), None, "other marks");
        let four = &["{4, 3, 2, 1}", "{4, 2, 3, 1}"];
        assert_eq!(found("{1, 2, 3…", four), Some(("{4, 3, 2, 1}", false)), "cut");
        assert_eq!(found("{1, 5, 3…", four), None, "another item, cut");
        assert!(as_stored("{1}", 8, Text("{2}"), || None).is_none(), "no longer generated");
    }
}
