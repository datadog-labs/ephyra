// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The human-readable report: `Display` for [`Report`].

use std::{
    fmt,
    time::Duration,
};

use super::{
    Causality,
    Failure,
    FailureKind,
    Report,
    Status,
};

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.status {
            Status::Passed => {
                writeln!(
                    f,
                    "ephyra: ok, {} tests passed ({} distinct, {} discarded) in {}, seed {}{}",
                    self.tests,
                    self.unique_tests,
                    self.discarded,
                    millis(self.duration),
                    self.seed,
                    if self.stopped_by_budget { " (stopped by the time budget)" } else { "" }
                )?;
            },
            Status::Failed => {
                let failure = self.failure.as_ref().expect("Failed reports carry a failure");
                writeln!(f, "ephyra: FAILED, {}", what(failure))?;
                if let Some(location) = &failure.location {
                    writeln!(f, "  at {location}")?;
                }
                let best = failure.counterexample();
                writeln!(
                    f,
                    "  counterexample: {}{}",
                    best.debug,
                    if best.truncated { " (truncated)" } else { "" }
                )?;
                if failure.shrunk.is_some() {
                    writeln!(
                        f,
                        "  original:       {}{} (size {}, iteration {})",
                        failure.original.debug,
                        if failure.original.truncated { " (truncated)" } else { "" },
                        failure.size,
                        failure.iteration
                    )?;
                } else {
                    writeln!(
                        f,
                        "  found at size {}, iteration {}",
                        failure.size, failure.iteration
                    )?;
                }
                if failure.flaky {
                    writeln!(
                        f,
                        "  FLAKY: the property did not fail the same way when re-run on this input"
                    )?;
                }
                if let Some(shrink) = &failure.shrink {
                    writeln!(
                        f,
                        "  shrinking: {} of {} candidates accepted in {}{}",
                        shrink.accepted,
                        shrink.steps,
                        millis(shrink.duration),
                        if shrink.budget_exhausted { " (budget exhausted)" } else { "" }
                    )?;
                }
                writeln!(
                    f,
                    "  after {} passing tests ({} distinct, {} discarded) in {}",
                    self.tests,
                    self.unique_tests,
                    self.discarded,
                    millis(self.duration)
                )?;
                match failure.replay {
                    Some(point) => writeln!(f, "  reproduce: EPHYRA_REPLAY={point}")?,
                    None => {
                        writeln!(
                            f,
                            "  reproduce: EPHYRA_SEED={} (found in the targeted phase)",
                            self.seed
                        )?
                    },
                }
                writeln!(f, "  or rerun the whole search: Config::default().seed({})", self.seed)?;
                if let Some(file) = &self.regressions.file
                    && self.regressions.saved
                {
                    writeln!(f, "  remembered in {}", file.display())?;
                }
                write_failures(f, self)?;
            },
            Status::GaveUp => {
                writeln!(
                    f,
                    "ephyra: GAVE UP after {} discarded inputs ({} tests passed), seed {}",
                    self.discarded, self.tests, self.seed
                )?;
            },
            Status::TimedOut => {
                writeln!(
                    f,
                    "ephyra: TIMED OUT after {} ({} tests passed, {} discarded), seed {}",
                    millis(self.duration),
                    self.tests,
                    self.discarded,
                    self.seed
                )?;
            },
            Status::Aborted => {
                let abort = self.abort.as_ref().expect("Aborted reports carry an abort");
                writeln!(f, "ephyra: ABORTED, the generator panicked: {}", abort.message)?;
                if let Some(location) = &abort.location {
                    writeln!(f, "  at {location}")?;
                }
                writeln!(f, "  iteration {}, seed {}", abort.iteration, self.seed)?;
            },
        }
        if let Some(replay) = &self.replay {
            writeln!(
                f,
                "  replay point {} {}",
                replay.point,
                if replay.reproduced { "reproduced the failure" } else { "passed" }
            )?;
        }
        if self.regressions.replayed > 0 {
            writeln!(
                f,
                "  regressions: {} stored input(s) replayed{}",
                self.regressions.replayed,
                if self.regressions.stale > 0 {
                    format!(", {} stale", self.regressions.stale)
                } else {
                    String::new()
                }
            )?;
        }
        if let Some(target) = &self.target {
            if target.scored {
                writeln!(
                    f,
                    "  target: best score {} reached by {} ({} mutated inputs, {} improvements)",
                    target.best_score, target.best_debug, target.iterations, target.improvements
                )?;
            } else {
                writeln!(
                    f,
                    "  guided search: {} mutated inputs from coverage-novel seeds",
                    target.iterations
                )?;
            }
        }
        if let Some(causality) = &self.causality {
            write_causality(f, causality)?;
        }
        #[cfg(feature = "coverage")]
        if let Some(coverage) = &self.coverage {
            coverage.write_report(f)?;
        }
        if let Some(profraw) = &self.profraw {
            writeln!(f, "  profraw written to {}", profraw.display())?;
        }
        if let Some(page) = &self.coverage_html {
            writeln!(f, "  coverage grid written to {}", page.display())?;
        }
        #[cfg(feature = "coverage")]
        if let Some(locate) = &self.locate {
            locate.write_report(f)?;
        }
        if !self.covers.is_empty() {
            let covers =
                self.covers.iter().map(|(label, share)| format!("{label} {:.0}%", share * 100.0));
            writeln!(f, "  cover: {}", joined(covers, usize::MAX))?;
        }
        if !self.discard_reasons.is_empty() {
            let reasons =
                self.discard_reasons.iter().map(|(reason, count)| format!("{reason} x{count}"));
            writeln!(f, "  discards: {}", joined(reasons, usize::MAX))?;
        }
        if !self.labels.is_empty() && self.tests > 0 {
            let labels = self.labels.iter().map(|(label, count)| {
                format!("{label} {:.0}%", *count as f64 * 100.0 / self.tests as f64)
            });
            writeln!(f, "  labels: {}", joined(labels, usize::MAX))?;
        }
        for (name, feature) in &self.features {
            writeln!(
                f,
                "  feature {name}: {} values, min {}, median {}, p90 {}, max {}",
                feature.count,
                number(feature.min),
                number(feature.median),
                number(feature.p90),
                number(feature.max)
            )?;
            if feature.histogram.len() > 1 {
                let buckets: Vec<String> = feature
                    .histogram
                    .iter()
                    .filter(|b| b.count > 0)
                    .map(|b| {
                        let range = if b.from == b.to {
                            number(b.from)
                        } else {
                            format!("{}-{}", number(b.from), number(b.to))
                        };
                        format!("{range} {:.0}%", b.count as f64 * 100.0 / feature.count as f64)
                    })
                    .collect();
                writeln!(f, "    histogram: {}", buckets.join(", "))?;
            }
        }
        if let Some(timing) = &self.timing {
            writeln!(
                f,
                "  timing: property median {}, p90 {}, max {}; generation median {}, p90 {}, max {}",
                human(timing.prop.median),
                human(timing.prop.p90),
                human(timing.prop.max),
                human(timing.generation.median),
                human(timing.generation.p90),
                human(timing.generation.max)
            )?;
            if let Some(slowest) = &timing.slowest {
                writeln!(f, "    slowest input: {slowest}")?;
            }
        }
        for sample in &self.samples {
            writeln!(
                f,
                "  sample ({}, iteration {}, size {}): {}",
                sample.origin, sample.iteration, sample.size, sample.debug
            )?;
        }
        for warning in &self.health {
            writeln!(f, "  warning [{}]: {}", warning.kind, warning.message)?;
        }
        Ok(())
    }
}

/// What went wrong, for the first line of a failure.
fn what(failure: &Failure) -> String {
    match (failure.kind, &failure.message) {
        (FailureKind::False, _) => "property returned false".to_string(),
        (FailureKind::Error, Some(m)) => format!("property failed: {m}"),
        (FailureKind::Error, None) => "property failed".to_string(),
        (FailureKind::Panic, Some(m)) => format!("property panicked: {m}"),
        (FailureKind::Panic, None) => "property panicked".to_string(),
    }
}

/// The distinct failures of a run that looked for several: the first
/// one's signature and count (its details are above), then a short block
/// for each of the others.
fn write_failures(f: &mut fmt::Formatter<'_>, report: &Report) -> fmt::Result {
    let total = report.failures.len();
    let times = |count: u64| if count == 1 { "once".to_string() } else { format!("{count} times") };
    for (i, found) in report.failures.iter().enumerate() {
        if i == 0 {
            writeln!(
                f,
                "  failure 1 of {total} (seen {}), signature: {}",
                times(found.count),
                found.signature
            )?;
            continue;
        }
        let failure = &found.failure;
        writeln!(
            f,
            "  failure {} of {total} (seen {}): {}",
            i + 1,
            times(found.count),
            what(failure)
        )?;
        writeln!(f, "    signature: {}", found.signature)?;
        let best = failure.counterexample();
        writeln!(
            f,
            "    counterexample: {}{}",
            best.debug,
            if best.truncated { " (truncated)" } else { "" }
        )?;
        if failure.flaky {
            writeln!(f, "    FLAKY: the property did not fail the same way when re-run")?;
        }
        match failure.replay {
            Some(point) => writeln!(f, "    reproduce: EPHYRA_REPLAY={point}")?,
            None => {
                writeln!(
                    f,
                    "    reproduce: EPHYRA_SEED={} (found in the targeted phase)",
                    report.seed
                )?
            },
        }
    }
    Ok(())
}

/// The causality section: the rate at both ends, the curve as a sparkline,
/// and the largest rise.
fn write_causality(f: &mut fmt::Formatter<'_>, causality: &Causality) -> fmt::Result {
    let percent = |rate: f64| format!("{:.0}%", rate * 100.0);
    let (Some(first), Some(last)) = (causality.points.first(), causality.points.last()) else {
        return Ok(());
    };
    writeln!(
        f,
        "  causality over {} steps: the failure recurs in {} of continuations after {} steps, {} after {}",
        causality.steps,
        percent(first.rate()),
        first.prefix,
        percent(last.rate()),
        last.prefix
    )?;
    const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let curve: String =
        causality.points.iter().map(|p| BARS[((p.rate() * 7.0).round() as usize).min(7)]).collect();
    let branches: u64 = causality.points.iter().map(|p| p.branches).sum();
    writeln!(f, "    {curve}  ({} prefixes, {branches} continuations)", causality.points.len())?;
    if let Some(jump) = &causality.jump {
        let at = if jump.to == jump.from + 1 {
            format!("at step {}", jump.to)
        } else {
            format!("between steps {} and {}", jump.from + 1, jump.to)
        };
        writeln!(
            f,
            "    the rate rose most {at}: {} after {} steps, {} after {}",
            percent(jump.before),
            jump.from,
            percent(jump.after),
            jump.to
        )?;
    }
    Ok(())
}

fn millis(d: Duration) -> String {
    format!("{:.1} ms", d.as_secs_f64() * 1000.0)
}

/// A duration in the unit that reads best.
fn human(d: Duration) -> String {
    let micros = d.as_secs_f64() * 1e6;
    if micros < 1000.0 {
        format!("{micros:.0} µs")
    } else if micros < 1e6 {
        format!("{:.1} ms", micros / 1000.0)
    } else {
        format!("{:.2} s", micros / 1e6)
    }
}

/// `items` joined by ", ", keeping the first `limit` and counting the rest.
pub(crate) fn joined(items: impl ExactSizeIterator<Item = String>, limit: usize) -> String {
    let total = items.len();
    let mut out = items.take(limit).collect::<Vec<_>>().join(", ");
    if total > limit {
        out.push_str(&format!(" (+{} more)", total - limit));
    }
    out
}

/// A feature value without a trailing `.0` for integers.
fn number(x: f64) -> String {
    if x.fract() == 0.0 && x.abs() < 1e15 { format!("{x:.0}") } else { format!("{x:.3}") }
}
