// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! When a run stops and what it then reports, through `check`, the shipped
//! loops and loops written by hand.

use std::time::Duration;

use ephyra::{
    Config,
    GenExt,
    Run,
    Status,
    TestResult,
    check,
    config::ReplayPoint,
    gens,
    loops,
};

fn cfg() -> Config {
    Config::default().seed(1).tests(100).no_regressions()
}

/// More tests than a run with a time limit gets to here.
fn long() -> Config {
    cfg().tests(100_000)
}

fn discard(_: u8) -> TestResult {
    TestResult::discard("always")
}

#[test]
fn the_shortest_budget_tries_one_input() {
    for budget in [Duration::ZERO, Duration::from_micros(800)] {
        let report = check(&long().budget(budget), gens::int(0u8..=9), |_| false);
        assert_eq!(report.status, Status::Failed, "{report}");
        let report = check(&long().budget(budget), gens::int(0u8..=9), |_| true);
        assert_eq!(report.status, Status::Passed, "{report}");
        assert!(report.stopped_by_budget && report.tests > 0, "{report}");
    }
}

#[test]
fn a_loop_that_finds_no_input_to_try_ends_at_the_budget() {
    let mut run = Run::new(&long().budget(Duration::ZERO), gens::int(0u8..=9), |_| true);
    let mut asked = 0;
    while asked < 1_000 && run.wants_more() {
        asked += 1;
        let Some(parent) = run.pool_pick() else { continue };
        let Some(input) = run.mutate(&parent) else { continue };
        run.test(&input);
    }
    let report = run.finish();
    assert!(report.stopped_by_budget, "asked {asked} times: {report}");
}

#[test]
fn a_run_whose_every_input_is_discarded_does_not_pass() {
    // The budget ends the run long before the discard limit.
    let short = long().budget(Duration::ZERO);
    let by_property = check(&short, gens::int(0u8..=9), discard);
    let by_generator = check(&short, gens::int(0u8..=9).filter("never", |_| false), |_| true);

    // Loops that end below the discard limit, or whose limit is 0.
    let mut run = Run::new(&cfg(), gens::int(0u8..=9), discard);
    for _ in 0..10 {
        let Some(input) = run.draw() else { continue };
        run.test(&input);
    }
    let by_hand = run.finish();
    let mut run = Run::new(&cfg().tests(0), gens::int(0u8..=9), discard);
    loops::fuzz(&mut run, &mut loops::Fifo::new(), 10);
    let shipped = run.finish();

    for report in [&by_property, &by_generator, &by_hand, &shipped] {
        assert_eq!(report.status, Status::GaveUp, "{report}");
        assert!(report.tests == 0 && report.discarded > 0, "{report}");
        assert!(report.to_string().contains("(0 tests passed)"), "{report}");
    }
    assert!(by_property.to_string().contains("stopped by the time budget"), "{by_property}");
}

#[test]
fn one_passing_test_or_no_attempt_at_all_is_still_a_pass() {
    let mut run = Run::new(&cfg(), gens::int(0u8..=9), |x: u8| {
        ephyra::assume!(x == 5);
        TestResult::Pass
    });
    loops::fuzz(&mut run, &mut loops::Fifo::new(), 100);
    let report = run.finish();
    assert_eq!(report.status, Status::Passed, "{report}");
    assert!(report.tests > 0 && report.discarded > report.tests, "{report}");

    // `tests(0)` asks for no search; a replayed input is not one either.
    let point = ReplayPoint { seed: 1, iteration: 3 };
    for config in [cfg().tests(0), cfg().tests(0).replay(point)] {
        let report = check(&config, gens::int(0u8..=9), discard);
        assert_eq!(report.status, Status::Passed, "{report}");
        assert_eq!((report.iterations, report.tests, report.discarded), (0, 0, 0), "{report}");
    }
}

/// Failures do not end the run; a budget does if the tests do not.
fn failures(n: usize) -> Config {
    cfg().failures(n).budget(Duration::from_secs(20))
}

#[test]
fn failing_inputs_use_up_the_tests_when_one_failure_does_not_end_the_run() {
    let all_fail = check(&failures(2), gens::int(0u8..=9), |_| false);
    let half_fail = check(&failures(2), gens::int(0u8..=9), |x| x % 2 == 0);
    let mut run = Run::new(&failures(2), gens::int(0u8..=9), |x| x % 2 == 0);
    while run.wants_more() {
        let Some(input) = run.draw() else { continue };
        run.test(&input);
    }
    let by_hand = run.finish();
    for report in [&all_fail, &half_fail, &by_hand] {
        assert_eq!(report.status, Status::Failed, "{report}");
        assert!(!report.stopped_by_budget, "{report}");
        assert_eq!(report.iterations, 100, "{report}");
        assert_eq!(report.failures.len(), 1, "{report}");
        assert_eq!(report.tests + report.failures[0].count, 100, "{report}");
    }
    assert_eq!(all_fail.tests, 0, "{all_fail}");
    assert!(half_fail.tests > 0, "{half_fail}");
}

#[cfg(feature = "derive")]
#[ephyra::test(config = failures(2))]
#[should_panic(expected = "(seen 100 times)")]
fn the_test_attribute_ends_when_every_input_fails(_x: u8) -> bool {
    false
}

#[cfg(feature = "json")]
#[test]
fn replayed_regressions_do_not_use_up_the_tests() {
    let dir = std::env::temp_dir().join(format!("ephyra-stopping-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let config = |tests| {
        let config = Config::default().seed(1).tests(tests).failures(100);
        config.budget(Duration::from_secs(20)).name("small").regressions_dir(&dir)
    };
    // Every value is a failure of its own, so each is stored.
    let property = |x: u8| TestResult::fail(["none", "one", "two", "three"][usize::from(x)]);
    let first = check(&config(100), gens::int(0u8..=3), property);
    assert_eq!(first.failures.len(), 4, "{first}");
    let second = check(&config(3), gens::int(0u8..=3), property);
    assert_eq!(second.regressions.replayed, 4, "{second}");
    assert_eq!(second.iterations, 3, "the search still has its tests: {second}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// #102: the targeted phase gave up only past its own budget, so a property that called
/// `target` passed with 129 discards under a limit of 10.
#[test]
fn the_targeted_phase_keeps_the_runs_discard_limit() {
    let config = Config::default().seed(2).tests(10).max_discard_ratio(1.0).no_regressions();
    let report = check(&config, ephyra::gens::int(0u32..=1000), |x: u32| {
        ephyra::target(f64::from(x));
        if x % 2 == 1 { TestResult::discard("odd") } else { TestResult::Pass }
    });
    assert!(
        report.status != Status::Passed || report.discarded <= 10,
        "{:?} with {} discards",
        report.status,
        report.discarded
    );
}
