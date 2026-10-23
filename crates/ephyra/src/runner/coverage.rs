// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The coverage side of a run: per-input tracking, the targets, fault
//! localization, the grid page and the coverage part of the report.

use std::{
    collections::BTreeMap,
    fmt::Debug,
};

#[cfg(feature = "coverage")]
use {
    super::exec::{
        At,
        Signature,
    },
    crate::{
        Source,
        report::Location,
        source::mix,
    },
};
use {
    super::{
        Outcome,
        Run,
        Testable,
        panic::PanicInfo,
    },
    crate::Gen,
};

/// Per-run coverage state: the tracker, the fault-localization spectrum and
/// the signature of the failure being localized.
#[cfg(feature = "coverage")]
#[derive(Default)]
pub(super) struct CoverageState {
    pub(super) tracker: Option<crate::coverage::tracker::Tracker>,
    pub(super) spectrum: Option<crate::coverage::locate::Spectrum>,
    pub(super) targets: Option<crate::coverage::targets::Targets>,
    pub(super) grid: Option<crate::coverage::grid::Grid>,
    pub(super) signature: Option<Signature>,
}

#[cfg(feature = "coverage")]
impl CoverageState {
    pub(super) fn begin(&self) -> Option<crate::coverage::Snapshot> {
        self.tracker.as_ref().and_then(|t| t.begin())
    }

    /// Record what an input did: whether it reached new coverage and, when
    /// asked, the source lines it executed. Passing runs and runs failing
    /// like the first failure feed the spectrum; discards and other failures
    /// do not.
    pub(super) fn end(
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
pub(super) struct Covered {
    /// Counters, hit-count classes and targets reached for the first time.
    pub(super) novelty: u32,
    pub(super) lines: Option<BTreeMap<String, Vec<u32>>>,
}

#[cfg(not(feature = "coverage"))]
#[derive(Default)]
pub(super) struct CoverageState {}

#[cfg(not(feature = "coverage"))]
impl CoverageState {
    pub(super) fn begin(&self) -> Option<()> {
        None
    }

    pub(super) fn end(
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
pub(super) fn crate_of_probe_name() -> &'static str {
    let probe = |x: i32| x;
    let _ = probe(0);
    std::any::type_name_of_val(&probe)
}

impl<G, R, P> Run<G, P>
where
    G: Gen,
    G::Value: Clone + Debug,
    R: Testable,
    P: FnMut(G::Value) -> R,
{
    /// Whether coverage-guided search is on and coverage is being collected.
    pub(super) fn guided(&self) -> bool {
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
    pub(super) fn enable_coverage(&mut self) -> Option<crate::coverage::Lease> {
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
                    self.exec.coverage.grid = crate::coverage::grid::Grid::new(&property_fns);
                    if self.exec.coverage.grid.is_none() {
                        self.report.warn(
                            "coverage_html",
                            "the binary carries no coverage mapping, so the coverage grid cannot be written",
                        );
                    }
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
    pub(super) fn resolve_targets(&mut self) -> Option<crate::coverage::targets::Targets> {
        let mut targets = self.config.coverage.targets.clone();
        if let Some(path) = self.config.coverage.targets_file.clone() {
            match crate::config::load_targets(&path) {
                Ok(more) => targets.extend(more),
                Err(e) => {
                    self.report
                        .warn("coverage_targets", format!("could not read the targets file: {e}"))
                },
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

    /// The fault-localization ranking as it stands: the functions and
    /// regions whose coverage separates the failing inputs tested so far
    /// from the passing ones, with the failure's panic site as a prior.
    /// `None` without a failure, or without coverage (`coverage` feature,
    /// instrumented build, [`Config::locate`](crate::Config::locate) on).
    #[cfg(feature = "coverage")]
    pub fn suspects(&self) -> Option<crate::coverage::Locate> {
        let spectrum = self.exec.coverage.spectrum.as_ref()?;
        let found = self.pending.as_ref()?;
        let signature = Signature::of(&found.result)?;
        let panic = signature.panic_site.as_ref().map(|(file, line)| (file.as_str(), *line));
        Some(spectrum.rank(panic, self.config.locate.top))
    }

    /// Sharpen the spectrum around the minimal failing input and rank it,
    /// with the failure's panic site as a prior.
    #[cfg(feature = "coverage")]
    pub(super) fn localize(
        &mut self,
        minimal: &G::Value,
        size: usize,
        at: At,
        location: Option<&Location>,
    ) {
        // Passing and failing neighbours of the minimal input sharpen the
        // spectrum where it matters.
        let neighbourhood = mix(self.report.seed, u64::MAX - 3);
        for i in 0..self.config.locate.neighbourhood as u64 {
            let mut source = Source::new(mix(neighbourhood, i), size);
            let mutant = self.generator.mutate(minimal, &mut source);
            if source.discarded().is_none() {
                self.run_property(mutant, At { origin: "neighbourhood", ..at });
            }
        }
        let panic = location.map(|l| (l.file.as_str(), l.line));
        self.report.locate =
            self.exec.coverage.spectrum.as_ref().map(|s| s.rank(panic, self.config.locate.top));
    }

    /// The coverage part of the report: the summary with its targets and
    /// warnings, the coverage grid page and the profile.
    #[cfg(feature = "coverage")]
    pub(super) fn finish_coverage(&mut self) {
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
                let missed: usize = summary.partial.iter().map(|(_, hit, total)| total - hit).sum();
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
}
