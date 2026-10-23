// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The per-run accumulators behind the report's distributions: passes and
//! discards, labels and features, timing and the sampled inputs.

use std::{
    collections::{
        BTreeMap,
        HashSet,
    },
    fmt::Debug,
};

use {
    super::{
        Run,
        Testable,
        labels,
    },
    crate::{
        Gen,
        config::ReplayPoint,
        source::hash_u01,
    },
};

/// Where a sampled or slow input can be rendered from later.
#[derive(Clone)]
pub(super) enum Provenance {
    /// Regenerate from the point (random-phase inputs).
    Point(ReplayPoint),
    /// Already rendered (mutants).
    Debug(String),
}

#[derive(Clone)]
pub(super) struct Candidate {
    pub(super) origin: crate::report::Origin,
    pub(super) size: usize,
    pub(super) iteration: u64,
    pub(super) source: Provenance,
}

/// The inputs shown as samples: the smallest and largest by size plus a
/// reservoir sample of the rest.
#[derive(Default)]
pub(super) struct Samples {
    pub(super) limit: usize,
    pub(super) smallest: Option<Candidate>,
    pub(super) largest: Option<Candidate>,
    pub(super) reservoir: Vec<Candidate>,
    pub(super) seen: u64,
}

impl Samples {
    pub(super) fn new(limit: usize) -> Samples {
        Samples { limit, ..Samples::default() }
    }

    /// Offer a passing input; `source` is only called when it is kept.
    pub(super) fn consider(
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
    pub(super) fn drain(&mut self) -> Vec<Candidate> {
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
pub(super) struct Stats {
    /// Debug hashes of passing inputs.
    pub(super) passed_seen: HashSet<u64>,
    /// Numeric feature values over passing tests.
    pub(super) features: BTreeMap<String, Vec<f64>>,
    pub(super) gen_micros: Vec<u32>,
    pub(super) prop_micros: Vec<u32>,
    pub(super) slowest: Option<(u32, Provenance)>,
    pub(super) distinct_growth: crate::report::Series,
    pub(super) samples: Samples,
}

impl Stats {
    pub(super) fn new(samples: usize) -> Stats {
        Stats { samples: Samples::new(samples), ..Stats::default() }
    }

    /// Note an input's property time; `source` is only called for a new
    /// maximum.
    pub(super) fn slower(
        &mut self,
        elapsed: std::time::Duration,
        source: impl FnOnce() -> Provenance,
    ) {
        let micros = micros(elapsed);
        if self.slowest.as_ref().is_none_or(|(best, _)| micros > *best) {
            self.slowest = Some((micros, source()));
        }
    }
}

/// Samples are for reading, so they are cut shorter than counterexamples.
pub(crate) const SAMPLE_CHARS: usize = 200;

/// Cut a rendering to `limit` characters, marking the cut.
pub(crate) fn cap(mut debug: String, limit: usize) -> String {
    if let Some((cut, _)) = debug.char_indices().nth(limit) {
        debug.truncate(cut);
        debug.push('…');
    }
    debug
}

pub(super) fn micros(d: std::time::Duration) -> u32 {
    d.as_micros().min(u128::from(u32::MAX)) as u32
}

impl<G, R, P> Run<G, P>
where
    G: Gen,
    G::Value: Clone + Debug,
    R: Testable,
    P: FnMut(G::Value) -> R,
{
    pub(super) fn record_discard(&mut self, reason: String) {
        self.report.discarded += 1;
        *self.report.discard_reasons.entry(reason).or_default() += 1;
    }

    pub(super) fn record_pass(
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

    /// Render an input kept by provenance; `None` when it can no longer be
    /// regenerated.
    pub(super) fn render_provenance(&mut self, source: Provenance) -> Option<String> {
        let limit = self.config.report.truncate_debug_at;
        match source {
            Provenance::Debug(debug) => Some(debug),
            Provenance::Point(point) => {
                self.regenerate(point).map(|value| cap(format!("{value:?}"), limit))
            },
        }
    }

    /// The final statistics: duration, distinct inputs, features, timing,
    /// samples and `cover` shares.
    pub(super) fn summarize(&mut self) {
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
    }
}
