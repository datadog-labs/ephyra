// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! One property: **a run's verdict matches what its inputs did.**
//!
//! The property under test is a script: each input value has an outcome (pass, fail,
//! discard, panic), returned in one of the shapes a property may return (`bool`, `()`,
//! `TestResult`, `Result<(), E>`, `Result<bool, E>`, `Result<TestResult, E>`,
//! `Result<Result<bool, E>, E>`). The run's settings are drawn too: the number of tests,
//! the discard ratio, `failures(n)`, a zero time budget, whether the property calls
//! `target`, and a generator whose `filter` can give up before a `map` that relies on it.
//! Whatever ends the run, the verdict must agree with what the property was seen to do:
//!
//! 1. the run ends;
//! 2. it fails if and only if some input it tested failed (a false return, a failing
//!    `TestResult`, an `Err`, or a panic, at any depth of `Result`);
//! 3. it passes only on evidence: at least one passing input (unless `tests(0)`), and no
//!    more discards than `max_discard_ratio * tests`;
//! 4. it is never `Aborted`: a panic in code after a `filter` that gave up is a discard.
//!
//! It cannot be written as a `check` over `check`: a `check` inside a property deadlocks on
//! instrumented builds (#61). The cases come from a seeded `Source` instead, and a failing
//! case prints everything needed to rerun it.
//!
//! See CONTRIBUTING.md, "Bugs: a property for the class, and a regression entry".
use std::{
    cell::RefCell,
    sync::mpsc,
    time::Duration,
};

use ephyra::{
    Config,
    GenExt,
    Report,
    Source,
    Status,
    TestResult,
    check,
    gens,
};

/// Input values are `0..VALUES`; each has its own outcome.
const VALUES: u32 = 16;
/// Cases tried.
const CASES: usize = 1000;
/// A run that has not ended by then is taken not to end.
const DEADLINE: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq)]
enum Outcome {
    Pass,
    Fail,
    Discard,
    Panic,
}

/// The return shapes, by the outcomes each can express (a shape that cannot discard
/// passes instead).
const SHAPES: &[&str] = &[
    "bool",
    "()",
    "TestResult",
    "Result<(), String>",
    "Result<bool, String>",
    "Result<TestResult, String>",
    "Result<Result<bool, String>, String>",
];

/// What an outcome means in a shape.
fn effective(shape: usize, outcome: Outcome) -> Outcome {
    let can_discard = matches!(SHAPES[shape], "TestResult" | "Result<TestResult, String>");
    if outcome == Outcome::Discard && !can_discard { Outcome::Pass } else { outcome }
}

#[derive(Clone, Debug)]
struct Case {
    seed: u64,
    shape: usize,
    outcomes: Vec<Outcome>,
    tests: u64,
    ratio: f64,
    failures: usize,
    zero_budget: bool,
    target: bool,
    /// `Some(allowed)`: the generator filters to the allowed values, then a `map` asserts
    /// the filter held (it panics on the value a filter that gave up returns).
    filtered: Option<Vec<bool>>,
}

fn draw_case(g: &mut Source) -> Case {
    let outcome = |g: &mut Source| {
        match g.int(0u8..10) {
            0..=4 => Outcome::Pass,
            5..=6 => Outcome::Discard,
            7..=8 => Outcome::Fail,
            _ => Outcome::Panic,
        }
    };
    // Three kinds of table, each common: one outcome throughout (where runs end oddly),
    // passes and discards only (runs that get as far as the targeted phase), and any mix.
    let outcomes: Vec<Outcome> = match g.int(0u8..3) {
        0 => vec![outcome(g); VALUES as usize],
        1 => (0..VALUES).map(|_| if g.bool() { Outcome::Pass } else { Outcome::Discard }).collect(),
        _ => (0..VALUES).map(|_| outcome(g)).collect(),
    };
    let allowed_share = g.int(0u8..=3);
    Case {
        seed: g.int(0u64..=u64::MAX),
        shape: g.int(0..SHAPES.len()),
        outcomes,
        tests: [0, 1, 2, 5, 10, 40][g.int(0usize..6)],
        ratio: [0.0, 0.5, 1.0, 2.0, 10.0][g.int(0usize..5)],
        failures: g.int(1usize..=3),
        zero_budget: g.int(0u8..4) == 0,
        target: g.bool(),
        filtered: g.bool().then(|| (0..VALUES).map(|_| g.int(0u8..3) < allowed_share).collect()),
    }
}

/// Run the case's property under its settings. Returns the report and the effective
/// outcome of every input the property was called on.
fn run(case: &Case) -> (Report, Vec<Outcome>) {
    let mut config = Config::default()
        .seed(case.seed)
        .tests(case.tests)
        .max_discard_ratio(case.ratio)
        .failures(case.failures)
        .no_regressions();
    if case.zero_budget {
        config = config.budget(Duration::ZERO);
    }
    let seen = RefCell::new(Vec::new());
    let outcome_of = |x: u32| {
        let o = case.outcomes[x as usize];
        seen.borrow_mut().push(effective(case.shape, o));
        if case.target {
            ephyra::target(f64::from(x));
        }
        o
    };
    let generator = match &case.filtered {
        None => gens::int(0..VALUES).boxed(),
        Some(allowed) => {
            let (a, b) = (allowed.clone(), allowed.clone());
            gens::int(0..VALUES)
                .filter("allowed", move |x| a[*x as usize])
                .map(move |x| {
                    assert!(b[x as usize], "the filter held");
                    x
                })
                .boxed()
        },
    };
    let report = match SHAPES[case.shape] {
        "bool" => {
            check(&config, generator, |x| {
                match outcome_of(x) {
                    Outcome::Fail => false,
                    Outcome::Panic => panic!("boom"),
                    _ => true,
                }
            })
        },
        "()" => {
            check(&config, generator, |x| {
                match outcome_of(x) {
                    Outcome::Fail => panic!("assertion failed"),
                    Outcome::Panic => panic!("boom"),
                    _ => {},
                }
            })
        },
        "TestResult" => {
            check(&config, generator, |x| {
                match outcome_of(x) {
                    Outcome::Pass => TestResult::Pass,
                    Outcome::Fail => TestResult::fail("no"),
                    Outcome::Discard => TestResult::discard("skip"),
                    Outcome::Panic => panic!("boom"),
                }
            })
        },
        "Result<(), String>" => {
            check(&config, generator, |x| {
                match outcome_of(x) {
                    Outcome::Fail => Err("no".to_string()),
                    Outcome::Panic => panic!("boom"),
                    _ => Ok(()),
                }
            })
        },
        "Result<bool, String>" => {
            check(&config, generator, |x| -> Result<bool, String> {
                match outcome_of(x) {
                    // the false value, not an Err: the verdict is in the Ok
                    Outcome::Fail => Ok(false),
                    Outcome::Panic => panic!("boom"),
                    _ => Ok(true),
                }
            })
        },
        "Result<TestResult, String>" => {
            check(&config, generator, |x| -> Result<TestResult, String> {
                match outcome_of(x) {
                    Outcome::Pass => Ok(TestResult::Pass),
                    Outcome::Fail => Ok(TestResult::fail("no")),
                    Outcome::Discard => Ok(TestResult::discard("skip")),
                    Outcome::Panic => panic!("boom"),
                }
            })
        },
        "Result<Result<bool, String>, String>" => {
            check(&config, generator, |x| -> Result<Result<bool, String>, String> {
                match outcome_of(x) {
                    Outcome::Fail if x % 2 == 0 => Ok(Ok(false)),
                    Outcome::Fail => Ok(Err("inner".to_string())),
                    Outcome::Panic => panic!("boom"),
                    _ => Ok(Ok(true)),
                }
            })
        },
        other => unreachable!("{other}"),
    };
    let seen = seen.into_inner();
    (report, seen)
}

/// Clauses 2-4 for one finished run.
fn verdict_agrees(case: &Case, report: &Report, seen: &[Outcome]) -> Result<(), String> {
    let failed = seen.iter().any(|o| matches!(o, Outcome::Fail | Outcome::Panic));
    let passes = seen.iter().filter(|o| **o == Outcome::Pass).count();
    let limit = (case.ratio * case.tests as f64) as u64;
    match report.status {
        Status::Aborted => Err(report.to_string().lines().next().unwrap_or("ABORTED").to_string()),
        Status::Failed if !failed => Err("FAILED, but no input it tested failed".into()),
        Status::Failed => Ok(()),
        _ if failed => Err(format!("{:?}, but an input failed (seen {seen:?})", report.status)),
        Status::Passed if case.tests > 0 && passes == 0 => {
            Err(format!(
                "PASSED without a passing input ({} tests, {} discarded)",
                report.tests, report.discarded
            ))
        },
        Status::Passed if report.discarded > limit => {
            Err(format!(
                "PASSED with {} discards, above the limit {} ({} x {} tests)",
                report.discarded, limit, case.ratio, case.tests
            ))
        },
        _ => Ok(()),
    }
}

/// Clause 1, then 2-4: the run, on a thread, must end before the deadline.
fn holds(case: &Case) -> Result<(), String> {
    let (tx, rx) = mpsc::channel();
    let c = case.clone();
    std::thread::spawn(move || {
        let (report, seen) = run(&c);
        let _ = tx.send(verdict_agrees(&c, &report, &seen));
    });
    match rx.recv_timeout(DEADLINE) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => Err(format!("did not end within {DEADLINE:?}")),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err("the check itself panicked".into()),
    }
}

#[test]
fn a_runs_verdict_matches_what_its_inputs_did() {
    let mut g = Source::new(63, 0);
    for i in 0..CASES {
        let case = draw_case(&mut g);
        if let Err(why) = holds(&case) {
            panic!("case {i}, {}: {why}\n{case:#?}", SHAPES[case.shape]);
        }
    }
}
