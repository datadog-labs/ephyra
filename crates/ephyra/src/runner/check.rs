//! The quality checks that `ephyra check` runs, through the tests that
//! `#[ephyra::generator]` and `#[ephyra::test(gen = ...)]` add: a
//! generator against its predicate, or a property's strategy against the
//! property's domain. Each is a full run (coverage, reports, regressions,
//! shrinking) whose property checks one generated value and its hooks.

use std::{
    cell::Cell,
    ops::ControlFlow,
    rc::Rc,
};

use {
    super::*,
    crate::BoxGen,
};

/// Shrink candidates checked per input.
const CANDIDATES: usize = 64;
/// Steps the first-candidate shrink chain may take before it must settle.
const CHAIN: usize = 1_000;
/// Mutants checked per input.
const MUTANTS: u64 = 4;

/// Check `generator` against `predicate`: every generated value, shrink
/// candidate, first-candidate chain step and mutant satisfies it, shrink
/// chains settle, and `metrics` stay finite (they are reported as
/// features). An implementation detail of the check test that
/// `#[ephyra::generator]` adds.
#[doc(hidden)]
pub fn __check_generator<G, S, M>(config: &Config, generator: G, predicate: S, metrics: M) -> Report
where
    G: Gen + 'static,
    G::Value: Clone + Debug + 'static,
    S: Fn(&G::Value) -> bool,
    M: Fn(&G::Value, usize) -> Vec<(&'static str, f64)>,
{
    let config = crate::verify::authored_by(config, &predicate);
    let domain = move |value: &G::Value| {
        match panic::catch(|| predicate(value)) {
            Ok(true) => None,
            Ok(false) => Some("the predicate returned false".to_string()),
            Err(info) => Some(format!("the predicate panicked: {}", info.message)),
        }
    };
    run_check(&config, generator, domain, Generated::MustSatisfy, metrics)
}

/// Check a property's strategy against the property's domain: an input the
/// property discards (through `assume!`) is outside it. Generated values
/// outside the domain are counted as discards; shrink candidates, chain
/// steps and mutants outside it fail the check. A property failure or panic
/// is an applicable input, not a problem with the strategy. An
/// implementation detail of the check test that
/// `#[ephyra::test(gen = ...)]` adds.
#[doc(hidden)]
pub fn __check_strategy<G, R, P, M>(
    config: &Config,
    generator: G,
    mut property: P,
    metrics: M,
) -> Report
where
    G: Gen + 'static,
    G::Value: Clone + Debug + 'static,
    R: Testable,
    P: FnMut(G::Value) -> R,
    M: Fn(&G::Value, usize) -> Vec<(&'static str, f64)>,
{
    let config = crate::verify::authored_by(config, &property);
    let domain = move |value: &G::Value| {
        match evaluate(&mut property, value.clone(), false).0 {
            Ok(Outcome::Discard(reason)) => Some(format!("the property discarded it: {reason}")),
            Ok(_) | Err(_) => None,
        }
    };
    run_check(&config, generator, domain, Generated::MayDiscard, metrics)
}

/// What a generated value outside the domain means.
#[derive(Clone, Copy)]
enum Generated {
    /// A failure: the generator promises the predicate.
    MustSatisfy,
    /// A discard: a strategy may approximate the domain.
    MayDiscard,
}

/// What the check property learns from the loop that drives it.
#[derive(Default)]
struct Progress {
    /// The run's seed, for mutation randomness.
    seed: Cell<u64>,
    /// The size the current input was drawn at.
    size: Cell<usize>,
    /// Inputs come from the generator (not from shrinking a failure).
    drawing: Cell<bool>,
    /// The failure being shrunk is a generated value outside the domain, so
    /// shrinking moves among such values; otherwise it stays inside the
    /// domain.
    outside: Cell<bool>,
}

fn run_check<G, D, M>(
    config: &Config,
    generator: G,
    mut domain: D,
    generated: Generated,
    metrics: M,
) -> Report
where
    G: Gen + 'static,
    G::Value: Clone + Debug + 'static,
    D: FnMut(&G::Value) -> Option<String>,
    M: Fn(&G::Value, usize) -> Vec<(&'static str, f64)>,
{
    let shared = BoxGen::new(generator);
    let hooks = shared.clone();
    let progress = Rc::new(Progress { drawing: Cell::new(true), ..Progress::default() });
    let seen = progress.clone();
    let filter_max_attempts = config.filter_max_attempts;
    let mut mutations = 0u64;
    let mut run = Run::new(config, shared, move |value: G::Value| {
        if let Some(reason) = domain(&value) {
            return match generated {
                Generated::MustSatisfy if seen.drawing.get() || seen.outside.get() => {
                    seen.outside.set(true);
                    TestResult::fail(format!("generated value: {reason}"))
                },
                _ => TestResult::discard(reason),
            };
        }
        let size = seen.size.get();
        let sources = (mutations..mutations + MUTANTS).map(|index| {
            Source::new(mix(seen.seed.get(), index), size)
                .with_filter_max_attempts(filter_max_attempts)
        });
        mutations += MUTANTS;
        let checked = shrinks(&value, &hooks, &mut domain)
            .and_then(|()| mutants(&value, &hooks, &mut domain, sources))
            .and_then(|()| record_metrics(metrics(&value, size)));
        match checked {
            Ok(()) => TestResult::Pass,
            Err(problem) => TestResult::fail(problem),
        }
    });
    progress.seed.set(run.report().seed);
    if !run.remembered() {
        while run.wants_more() {
            let Some(input) = run.draw() else { continue };
            progress.size.set(input.size());
            run.test(&input);
        }
    }
    progress.drawing.set(false);
    run.finish()
}

/// The first `CANDIDATES` shrink candidates of `value` differ from it and
/// stay in the domain, and the first-candidate chain from it stays in the
/// domain, never revisits a value and settles within `CHAIN` steps.
fn shrinks<T: Debug>(
    value: &T,
    hooks: &BoxGen<T>,
    domain: &mut impl FnMut(&T) -> Option<String>,
) -> Result<(), String> {
    let parent = format!("{value:?}");
    let mut candidates = Vec::new();
    let _ = hooks.shrink(value, &mut |candidate| {
        candidates.push(candidate);
        if candidates.len() < CANDIDATES {
            ControlFlow::Continue(())
        } else {
            ControlFlow::Break(())
        }
    });
    for candidate in &candidates {
        let rendered = format!("{candidate:?}");
        if rendered == parent {
            return Err("the shrinker returned its input unchanged".to_string());
        }
        if let Some(reason) = domain(candidate) {
            return Err(format!("shrink candidate {}: {reason}", cap(rendered, SAMPLE_CHARS)));
        }
    }
    let mut seen = HashSet::from([parent]);
    let Some(mut current) = candidates.into_iter().next() else { return Ok(()) };
    for _ in 0..CHAIN {
        let rendered = format!("{current:?}");
        if !seen.insert(rendered.clone()) {
            return Err(format!(
                "the first-candidate shrink chain revisits {}",
                cap(rendered, SAMPLE_CHARS)
            ));
        }
        let Some(next) = shrink::first_candidate(|visit| hooks.shrink(&current, visit)) else {
            return Ok(());
        };
        if let Some(reason) = domain(&next) {
            return Err(format!(
                "first-candidate shrink chain step {}: {reason}",
                cap(format!("{next:?}"), SAMPLE_CHARS)
            ));
        }
        current = next;
    }
    Err(format!(
        "the first-candidate shrink chain did not settle within {CHAIN} steps; last value {}",
        cap(format!("{current:?}"), SAMPLE_CHARS)
    ))
}

/// A mutant of `value` from each of `sources` is drawn without a discard
/// and stays in the domain.
fn mutants<T: Debug>(
    value: &T,
    hooks: &BoxGen<T>,
    domain: &mut impl FnMut(&T) -> Option<String>,
    sources: impl Iterator<Item = Source>,
) -> Result<(), String> {
    for mut source in sources {
        let mutant = hooks.mutate(value, &mut source);
        if let Some(reason) = source.discarded() {
            return Err(format!("the mutator discarded: {reason}"));
        }
        if let Some(reason) = domain(&mutant) {
            return Err(format!("mutant {}: {reason}", cap(format!("{mutant:?}"), SAMPLE_CHARS)));
        }
    }
    Ok(())
}

/// Report each metric as a feature; a non-finite one is a problem.
fn record_metrics(measured: Vec<(&'static str, f64)>) -> Result<(), String> {
    for (name, value) in measured {
        if !value.is_finite() {
            return Err(format!("metric `{name}` is {value}"));
        }
        labels::feature(name, value);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::{
            GenExt,
            gens,
        },
    };

    fn cfg() -> Config {
        Config::default().seed(1).tests(200).no_regressions()
    }

    fn no_metrics<T>(_: &T, _: usize) -> Vec<(&'static str, f64)> {
        Vec::new()
    }

    fn problem(report: &Report) -> String {
        assert_eq!(report.status, Status::Failed, "{report}");
        report.failure.as_ref().and_then(|f| f.message.clone()).unwrap_or_default()
    }

    #[test]
    fn a_generator_that_keeps_its_predicate_passes_and_reports_its_metrics() {
        let report = __check_generator(
            &cfg(),
            gens::int(0u8..=9),
            |v| *v <= 9,
            |v, size| vec![("value", f64::from(*v)), ("size", size as f64)],
        );
        report.assert_passed();
        assert_eq!(report.tests, 200);
        assert_eq!(report.features["value"].count, 200);
        assert!(report.features.contains_key("size"));
    }

    #[test]
    fn generated_values_outside_the_predicate_fail_and_shrink_among_themselves() {
        let report = __check_generator(&cfg(), gens::int(5u8..=9), |v| *v < 5, no_metrics);
        assert!(problem(&report).contains("generated value: the predicate returned false"));
        assert_eq!(report.counterexample().unwrap().debug, "5");
    }

    #[test]
    fn hooks_that_leave_the_predicate_fail_and_shrink_inside_it() {
        let at_least_one = |v: &u8| *v >= 1;
        let candidate = gens::constant(2u8).with_shrink(|_, visit| visit(0));
        let report = __check_generator(&cfg(), candidate, at_least_one, no_metrics);
        assert!(problem(&report).contains("shrink candidate 0"), "{report}");
        assert_eq!(report.counterexample().unwrap().debug, "2");

        // 3's own candidate is fine; its chain reaches 0 through 2. Shrinking
        // then finds 2, whose candidate is 0 itself.
        let chain = gens::constant(3u8)
            .with_shrink(|v, visit| {
                match v {
                    3 => visit(2),
                    2 => visit(0),
                    _ => ControlFlow::Continue(()),
                }
            })
            .boxed();
        let mut unshrunk = cfg();
        unshrunk.shrink.enabled = false;
        let report = __check_generator(&unshrunk, chain.clone(), |v| *v >= 2, no_metrics);
        assert!(problem(&report).contains("shrink chain step 0"), "{report}");
        let report = __check_generator(&cfg(), chain, |v| *v >= 2, no_metrics);
        assert!(problem(&report).contains("shrink candidate 0"), "{report}");
        assert_eq!(report.counterexample().unwrap().debug, "2");

        let mutant = gens::constant(2u8).with_mutate(|_, _| 0);
        let report = __check_generator(&cfg(), mutant, at_least_one, no_metrics);
        assert!(problem(&report).contains("mutant 0"), "{report}");
    }

    #[test]
    fn shrink_chains_must_settle() {
        let cycle = gens::constant(1u8).with_shrink(|v, visit| visit(if *v == 1 { 2 } else { 1 }));
        let report = __check_generator(&cfg(), cycle, |_| true, no_metrics);
        assert!(problem(&report).contains("revisits"), "{report}");
    }

    #[test]
    fn non_finite_metrics_fail() {
        let report = __check_generator(
            &cfg(),
            gens::int(0u8..=9),
            |_| true,
            |_, _| vec![("ratio", f64::NAN)],
        );
        assert!(problem(&report).contains("metric `ratio` is NaN"), "{report}");
    }

    #[test]
    fn a_strategy_may_meet_property_failures_but_not_leave_the_domain() {
        let even = |v: i32| {
            if v % 2 == 0 {
                TestResult::fail("the property found a counterexample")
            } else {
                TestResult::discard("assume: v % 2 == 0")
            }
        };
        __check_strategy(&cfg(), gens::constant(2i32), even, no_metrics).assert_passed();

        let report = __check_strategy(&cfg(), gens::constant(3i32), even, no_metrics);
        assert_eq!(report.status, Status::GaveUp, "{report}");
        assert!(report.discard_reasons.keys().any(|r| r.contains("assume: v % 2 == 0")));

        let leaves = gens::constant(2i32).with_shrink(|_, visit| visit(3));
        let report = __check_strategy(&cfg(), leaves, even, no_metrics);
        assert!(problem(&report).contains("shrink candidate 3: the property discarded it"));
    }

    #[test]
    fn the_property_under_a_strategy_check_does_not_reach_the_check_report() {
        let labelled = |_: i32| {
            crate::label("from the property");
            crate::feature("property feature", 1.0);
            true
        };
        let report = __check_strategy(&cfg(), gens::int(0i32..=9), labelled, |v, _| {
            vec![("v", f64::from(*v))]
        });
        report.assert_passed();
        assert!(report.labels.is_empty(), "{:?}", report.labels);
        assert!(!report.features.contains_key("property feature"));
        assert_eq!(report.features["v"].count, 200);
    }
}
