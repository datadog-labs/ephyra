// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! One property: **what a failing report says is true.**
//!
//! A report is what a user acts on: they read the message, debug the counterexample, and
//! paste the replay point or the seed to get the failure back. Each case is a scripted
//! property over the values `0..16`: per value it passes, returns false, panics at one of
//! two sites written on one line, fails through `resume_unwind` after a panic it handled
//! itself, or fails through `resume_unwind` with a string payload; and the generator may
//! panic on some values. For every report:
//!
//! 1. the message is what the property does on the reported counterexample;
//! 2. the counterexample fails the way the input that was found did: the same kind and, for
//!    a panic, the same site (shrinking does not trade the failure for one nearby);
//! 3. replaying the reported point gives the same verdict and the same input, whatever the
//!    run's own seed (the point is the input: a run that only finds it again by searching
//!    from the same seed proves nothing);
//! 4. rerunning the whole search with the reported seed finds the same input, also when
//!    the report came from a replay;
//! 5. the checkers of hooks (`check_shrinker`, `check_mutator`) report counterexamples
//!    inside the support they were given.
//!
//! No `check` runs inside a property (it deadlocks instrumented builds, #61): the cases
//! come from a seeded `Source`, and a failing case prints everything needed to rerun it.
//!
//! See CONTRIBUTING.md, "Bugs: a property for the class, and a regression entry".
use std::panic::{
    catch_unwind,
    resume_unwind,
};

use ephyra::{
    BoxGen,
    Config,
    GenExt,
    ReplayPoint,
    Report,
    Source,
    Status,
    check,
    check_mutator,
    check_shrinker,
    gens,
};

const VALUES: u32 = 16;
const CASES: usize = 300;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Outcome {
    Pass,
    False,
    /// The first of two panics on one line.
    SiteA,
    /// The second of two panics on one line.
    SiteB,
    /// A panic the property catches itself, then a failure through `resume_unwind`.
    HandledThenResumed,
    /// A failure through `resume_unwind` with a `String` payload.
    Resumed,
}

/// The message a failing outcome reports on `x`.
fn message(outcome: Outcome, x: u32) -> Option<String> {
    match outcome {
        Outcome::Pass | Outcome::False => None,
        Outcome::SiteA => Some(format!("site a on {x}")),
        Outcome::SiteB => Some(format!("site b on {x}")),
        Outcome::HandledThenResumed => Some(format!("worker failed on {x}")),
        Outcome::Resumed => Some(format!("payload of {x}")),
    }
}

/// How a failure is told apart from another, as the docs define it: its kind and, for a
/// panic, its site. Messages are not compared (they usually show the input), so the two
/// failures through `resume_unwind`, which have no site on this thread, are one way to fail.
fn way(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Pass => "pass",
        Outcome::False => "returned false",
        Outcome::SiteA => "panic at site a",
        Outcome::SiteB => "panic at site b",
        Outcome::HandledThenResumed | Outcome::Resumed => "panic with no site",
    }
}

/// Two panic sites on one line, as `a.unwrap() + b.expect("b")` puts them.
#[rustfmt::skip]
fn two_sites(x: u32, a: bool, b: bool) { if a { panic!("site a on {x}") } else if b { panic!("site b on {x}") } }

/// The scripted property.
fn property(outcome: Outcome, x: u32) -> bool {
    match outcome {
        Outcome::Pass => true,
        Outcome::False => false,
        Outcome::SiteA => {
            two_sites(x, true, false);
            true
        },
        Outcome::SiteB => {
            two_sites(x, false, true);
            true
        },
        Outcome::HandledThenResumed => {
            assert!(catch_unwind(|| panic!("handled inside the property")).is_err());
            resume_unwind(Box::new(format!("worker failed on {x}")))
        },
        Outcome::Resumed => resume_unwind(Box::new(format!("payload of {x}"))),
    }
}

#[derive(Clone, Debug)]
struct Case {
    seed: u64,
    tests: u64,
    outcomes: Vec<Outcome>,
    /// Values the generator panics on.
    generator_panics: Vec<bool>,
}

fn draw_case(g: &mut Source) -> Case {
    const ALL: [Outcome; 6] = [
        Outcome::Pass,
        Outcome::False,
        Outcome::SiteA,
        Outcome::SiteB,
        Outcome::HandledThenResumed,
        Outcome::Resumed,
    ];
    // mostly passing tables, each with a few failures of one or two kinds, so that
    // shrinking has a way to go and other kinds of failure lie on its way
    let kinds = [ALL[g.int(1usize..6)], ALL[g.int(1usize..6)]];
    let outcomes = (0..VALUES)
        .map(|_| if g.int(0u8..3) == 0 { kinds[g.int(0usize..2)] } else { Outcome::Pass })
        .collect();
    let generator_panics = (0..VALUES).map(|_| g.int(0u8..8) == 0 && g.int(0u8..3) == 0).collect();
    Case {
        seed: g.int(0u64..=u64::MAX),
        tests: [5, 20, 100][g.int(0usize..3)],
        outcomes,
        generator_panics,
    }
}

fn generator(case: &Case) -> BoxGen<u32> {
    let panics = case.generator_panics.clone();
    gens::int(0..VALUES)
        .iso(
            move |x| {
                assert!(!panics[x as usize], "the generator cannot make {x}");
                x
            },
            |x: &u32| *x,
        )
        .boxed()
}

fn run(case: &Case, config: &Config) -> Report {
    let outcomes = case.outcomes.clone();
    check(config, generator(case), move |x| property(outcomes[x as usize], x))
}

fn config(case: &Case) -> Config {
    Config::default().tests(case.tests).no_regressions()
}

/// The value a counterexample's `Debug` text shows.
fn value(debug: &str) -> u32 {
    debug.parse().unwrap_or_else(|_| panic!("not a u32: {debug}"))
}

/// Clauses 1-4 for one case.
fn report_is_true(case: &Case) -> Result<(), String> {
    let first = run(case, &config(case).seed(case.seed));
    match first.status {
        Status::Failed => {
            let failure = first.failure.as_ref().expect("a failed run has a failure");
            let original = value(&failure.original.debug);
            let shrunk = failure.shrunk.as_ref().map_or(original, |s| value(&s.debug));
            let (found, reported) =
                (case.outcomes[original as usize], case.outcomes[shrunk as usize]);
            // 1
            if failure.message != message(reported, shrunk) {
                return Err(format!(
                    "message {:?}, but the property on {shrunk} fails with {:?}",
                    failure.message,
                    message(reported, shrunk)
                ));
            }
            if failure.flaky {
                return Err(format!(
                    "FLAKY, but the property is deterministic ({reported:?} on {shrunk})"
                ));
            }
            // 2
            if way(found) != way(reported) {
                return Err(format!(
                    "{original} failed with {found:?}, shrunk to {shrunk}, which fails with {reported:?}"
                ));
            }
            // 3
            let point = failure.replay.ok_or("a failure with no replay point")?;
            let replayed = run(case, &config(case).seed(case.seed.wrapping_add(1)).replay(point));
            let again = replayed.failure.as_ref().map(|f| value(&f.original.debug));
            if replayed.status != Status::Failed || again != Some(original) {
                return Err(format!(
                    "replaying {}:{} gave {:?} on {again:?}, not the failure on {original}",
                    point.seed, point.iteration, replayed.status
                ));
            }
            // 4: a report that came from a replay names the seed that reproduces it
            let from_replay = run(case, &config(case).replay(point));
            let rerun = run(case, &config(case).seed(from_replay.seed));
            let rerun_original = rerun.failure.as_ref().map(|f| value(&f.original.debug));
            if rerun_original != Some(original) {
                return Err(format!(
                    "a replay of {}:{} reported seed {}; rerunning it found {rerun_original:?}, not {original}",
                    point.seed, point.iteration, from_replay.seed
                ));
            }
            Ok(())
        },
        Status::Aborted => {
            let abort = first.abort.as_ref().expect("an aborted run has an abort");
            // 3, for an abort: the point it printed aborts again
            let point = ReplayPoint { seed: first.seed, iteration: abort.iteration };
            let replayed = run(case, &config(case).seed(case.seed.wrapping_add(1)).replay(point));
            if replayed.status != Status::Aborted {
                return Err(format!(
                    "aborted at {}:{} ({}), but replaying that point gave {:?}",
                    point.seed, point.iteration, abort.message, replayed.status
                ));
            }
            Ok(())
        },
        _ => Ok(()),
    }
}

/// Clause 5: a checker given a hook that leaves the support reports a counterexample inside
/// it. `escape` is the value the hook offers, inside or outside `lo..=hi`.
fn checkers_report_inside_the_support(g: &mut Source) -> Result<(), String> {
    let lo = g.int(0i32..=50);
    let hi = lo + g.int(0i32..=50);
    let escape = g.int(-20i32..=120);
    let inside = move |x: &i32| (lo..=hi).contains(x);
    let config = Config::default().seed(g.int(0u64..=u64::MAX)).tests(50).no_regressions();
    let shrinker = gens::int(lo..=hi).with_shrink(move |x, visit| {
        if *x != escape { visit(escape) } else { std::ops::ControlFlow::Continue(()) }
    });
    let mutator = gens::int(lo..=hi).with_mutate(move |_, _| escape);
    let reports = [
        ("check_shrinker", check_shrinker(&config, shrinker, inside, |x| x.unsigned_abs() as u64)),
        ("check_mutator", check_mutator(&config, mutator, inside)),
    ];
    for (name, report) in reports {
        let Some(failure) = &report.failure else { continue };
        let shown = failure.shrunk.as_ref().unwrap_or(&failure.original);
        let x: i32 = shown.debug.parse().map_err(|_| format!("{name}: {}", shown.debug))?;
        if !inside(&x) {
            return Err(format!("{name} on int({lo}..={hi}) reported {x}, outside the support"));
        }
    }
    Ok(())
}

#[test]
fn what_a_failing_report_says_is_true() {
    let mut g = Source::new(58, 0);
    for i in 0..CASES {
        let case = draw_case(&mut g);
        if let Err(why) = report_is_true(&case) {
            panic!("case {i}: {why}\n{case:?}");
        }
        if let Err(why) = checkers_report_inside_the_support(&mut g) {
            panic!("case {i}: {why}");
        }
    }
}
