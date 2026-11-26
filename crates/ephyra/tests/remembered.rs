// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The replay point and the stored regressions in loops that do not call
//! `Run::remembered` themselves.

use std::sync::atomic::{
    AtomicU32,
    Ordering,
};

use ephyra::{
    Config,
    Gen,
    Report,
    Run,
    Source,
    Status,
    check,
    config::ReplayPoint,
    gens,
    loops,
};

fn cfg() -> Config {
    Config::default().seed(2).tests(50).no_regressions()
}

fn ints() -> impl Gen<Value = u32> {
    gens::int(0u32..=999_999)
}

/// The point of one input, and a property that fails on that input alone.
fn rare() -> (ReplayPoint, impl Fn(u32) -> bool + Copy) {
    let mut run = Run::new(&cfg().seed(1), ints(), |_| true);
    let input = (0..40).filter_map(|_| run.draw()).last().expect("ints never discard");
    (input.replay().expect("a drawn input"), move |x| x != input.value)
}

/// One run of each kind of loop that leaves the replay to the library.
fn loops_over<P: Fn(u32) -> bool + Copy>(config: &Config, property: P) -> [Report; 4] {
    let mut by_hand = Run::new(config, ints(), property);
    while by_hand.wants_more() {
        let Some(input) = by_hand.draw() else { continue };
        by_hand.test(&input);
    }
    let by_hand = by_hand.finish();
    let mut fuzz = Run::new(config, ints(), property);
    loops::fuzz(&mut fuzz, &mut loops::Fifo::new(), 50);
    let fuzz = fuzz.finish();
    let mut target = Run::new(config, ints(), property);
    loops::target(&mut target, &mut loops::Heap::new(), 50);
    let target = target.finish();
    let mut perf_fuzz = Run::new(config, ints(), property);
    loops::perf_fuzz(&mut perf_fuzz, &mut loops::Heap::new(), 50);
    [by_hand, fuzz, target, perf_fuzz.finish()]
}

#[test]
fn every_loop_tests_the_replay_point_first() {
    let (point, property) = rare();
    for config in
        [cfg().replay(point), cfg().replay(point).tests(0), cfg().replay(point).failures(3)]
    {
        for report in loops_over(&config, property) {
            assert_eq!(report.status, Status::Failed, "{report}");
            assert!(report.replay.as_ref().is_some_and(|r| r.point == point && r.reproduced));
            assert_eq!(report.failure.as_ref().and_then(|f| f.replay), Some(point), "{report}");
            assert_eq!((report.iterations, report.tests), (0, 0), "no search: {report}");
        }
    }
    // A point that passes is not a test, and the search goes on.
    for report in loops_over(&cfg().replay(point), |_| true) {
        assert!(report.passed(), "{report}");
        assert!(report.replay.as_ref().is_some_and(|r| !r.reproduced));
        assert_eq!((report.iterations, report.tests), (50, 50), "{report}");
    }
}

/// A loop that tests an input before it asks whether the run wants one.
fn draw_first(
    config: &Config,
    generator: impl Gen<Value = u32>,
    property: impl Fn(u32) -> bool,
) -> Report {
    let mut run = Run::new(config, generator, property);
    loop {
        if let Some(input) = run.draw() {
            run.test(&input);
        }
        if !run.wants_more() {
            return run.finish();
        }
    }
}

#[test]
fn the_replay_point_is_tested_after_inputs_the_loop_tests_before_it_asks() {
    let (point, property) = rare();
    let report = draw_first(&cfg().replay(point), ints(), property);
    assert_eq!(report.failure.as_ref().and_then(|f| f.replay), Some(point), "{report}");
    assert_eq!((report.iterations, report.tests), (1, 1), "{report}");
}

#[test]
fn the_replay_point_is_not_tested_after_the_end_of_the_run() {
    let (point, _) = rare();
    let failed = draw_first(&cfg().replay(point), ints(), |_| false);
    let first = ReplayPoint { seed: 2, iteration: 0 };
    assert_eq!(failed.failure.as_ref().and_then(|f| f.replay), Some(first), "{failed}");
    // Only the first input panics, so the replay point would fail.
    let calls = AtomicU32::new(0);
    let generator = move |_: &mut Source| {
        assert!(calls.fetch_add(1, Ordering::Relaxed) > 0, "generator bug");
        7
    };
    let aborted = draw_first(&cfg().replay(point), generator, |_| false);
    assert_eq!(aborted.status, Status::Aborted, "{aborted}");
    for report in [failed, aborted] {
        assert!(report.replay.is_none(), "{report}");
        assert!(report.health.iter().any(|w| w.kind == "replay"), "{report}");
    }
}

#[test]
fn a_loop_that_never_tests_the_replay_point_warns() {
    let (point, property) = rare();
    let warned = |config: &Config, search: bool| {
        let mut run = Run::new(config, ints(), property);
        if let Some(input) = run.draw() {
            run.test(&input);
        }
        if search {
            run.wants_more();
        }
        let report = run.finish();
        report.health.iter().any(|w| w.kind == "replay" && w.message.contains(&point.to_string()))
    };
    assert!(warned(&cfg().replay(point), false));
    assert!(!warned(&cfg().replay(point), true));
    assert!(!warned(&cfg(), false));
    assert!(
        !check(&cfg().replay(point), ints(), property).health.iter().any(|w| w.kind == "replay")
    );
}

#[cfg(feature = "json")]
mod stored {
    use std::{
        cell::Cell,
        path::PathBuf,
    };

    use super::*;

    /// A regressions directory, removed when dropped.
    struct Dir(PathBuf);

    impl Dir {
        fn new(tag: &str) -> Self {
            let name = format!("ephyra-remembered-{tag}-{}", std::process::id());
            let dir = std::env::temp_dir().join(name);
            let _ = std::fs::remove_dir_all(&dir);
            Dir(dir)
        }

        fn config(&self) -> Config {
            Config::default().seed(2).tests(50).name("stored").regressions_dir(&self.0)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn every_loop_replays_the_stored_regressions_once() {
        let dir = Dir::new("loops");
        let (point, property) = rare();
        assert!(check(&dir.config().replay(point), ints(), property).regressions.saved);
        for report in loops_over(&dir.config(), property) {
            assert_eq!(report.failure.as_ref().and_then(|f| f.replay), Some(point), "{report}");
            assert_eq!((report.regressions.replayed, report.iterations), (1, 0), "{report}");
        }

        // Several loops on one run, with and without the explicit step.
        for explicit in [false, true] {
            let replays = Cell::new(0);
            let mut run = Run::new(&dir.config(), ints(), |x| {
                replays.set(replays.get() + u64::from(!property(x)));
                true
            });
            if explicit {
                assert!(!run.remembered());
            }
            loops::fuzz(&mut run, &mut loops::Fifo::new(), 20);
            while run.wants_more() {
                let Some(input) = run.draw() else { continue };
                run.test(&input);
            }
            loops::target(&mut run, &mut loops::Heap::new(), 20);
            assert!(!run.remembered());
            let report = run.finish();
            report.assert_passed();
            assert_eq!((report.regressions.replayed, report.tests), (1, 70), "{report}");
            assert_eq!(replays.get(), 1);
        }
    }

    #[test]
    fn with_several_failures_the_search_goes_on_after_the_stored_ones() {
        let dir = Dir::new("failures");
        let (point, property) = rare();
        check(&dir.config().replay(point), ints(), property);
        let both = |x: u32| {
            assert!(property(x), "the stored one");
            assert!(x.is_multiple_of(2), "odd");
            true
        };
        for report in loops_over(&dir.config().failures(3), both) {
            assert_eq!(report.failures.len(), 2, "{report}");
            assert_eq!(report.failures[0].failure.replay, Some(point), "{report}");
            assert_eq!(report.iterations, 50, "{report}");
        }
    }

    #[test]
    fn a_loop_that_never_replays_them_warns() {
        let dir = Dir::new("warning");
        let (point, property) = rare();
        let warnings = |config: &Config| {
            let mut run = Run::new(config, ints(), |_| false);
            if let Some(input) = run.draw() {
                run.test(&input);
            }
            let report = run.finish();
            assert!(!report.passed());
            report.health.iter().filter(|w| w.kind == "regressions").count()
        };
        // The failure of the run itself is saved, not left untested.
        assert_eq!(warnings(&dir.config().name("other")), 0);
        check(&dir.config().replay(point), ints(), property);
        assert_eq!(warnings(&dir.config()), 1);
        assert_eq!(warnings(&dir.config().no_regressions()), 0);
    }

    #[test]
    fn failures_that_the_run_itself_saved_are_not_stored_ones() {
        // Each value is a failure of its own, saved before the run ends.
        let property = |x: u32| ephyra::TestResult::fail(["none", "one", "two"][x as usize % 3]);
        for asks in [false, true] {
            let dir = Dir::new(if asks { "own-asks" } else { "own" });
            let mut run = Run::new(&dir.config().failures(10), ints(), property);
            for _ in 0..20 {
                let Some(input) = run.draw() else { continue };
                run.test(&input);
            }
            while asks && run.wants_more() {
                let Some(input) = run.draw() else { continue };
                run.test(&input);
            }
            let report = run.finish();
            let failed: u64 = report.failures.iter().map(|f| f.count).sum();
            assert_eq!((report.failures.len(), failed), (3, report.iterations), "{report}");
            assert_eq!(report.regressions.replayed, 0, "{report}");
            assert!(!report.health.iter().any(|w| w.kind == "regressions"), "{report}");
        }
    }
}
