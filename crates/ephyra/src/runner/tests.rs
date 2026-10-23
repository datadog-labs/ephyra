// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

use std::cell::Cell;

#[cfg(feature = "coverage")]
use super::coverage::{
    crate_of,
    crate_of_probe_name,
};
use {
    super::{
        stats::{
            Provenance,
            Samples,
        },
        *,
    },
    crate::{
        GenExt,
        Source,
        classify,
        gens,
        report::FailureKind,
        source::hash_u01,
    },
};

fn cfg() -> Config {
    Config::default().seed(1).tests(64).no_regressions()
}

#[test]
fn mutation_time_counts_as_generation_time() {
    let slow = gens::int(0i32..=100).with_mutate(|value, _| {
        std::thread::sleep(std::time::Duration::from_millis(2));
        *value
    });
    let mut config = cfg().tests(20);
    config.target.tests = 20;
    let report = check(&config, slow, |x| {
        crate::target(f64::from(x));
        true
    });
    let iterations = report.target.as_ref().expect("the targeted phase ran").iterations;
    // Most targeted iterations mutate (the rest explore), each for 2 ms.
    assert!(
        report.gen_time >= std::time::Duration::from_millis(iterations),
        "{:?} over {iterations} targeted iterations",
        report.gen_time
    );
}

#[test]
fn passing_property() {
    let report = check(&cfg(), gens::vec(gens::int(0..=9)), |v: Vec<i32>| v.len() <= 100);
    assert_eq!(report.status, Status::Passed);
    assert_eq!(report.tests, 64);
    assert_eq!(report.discarded, 0);
    assert_eq!(report.iterations, 64);
    assert_eq!(report.sizes.keys().copied().collect::<Vec<_>>(), [0, 1, 2, 3, 4, 5, 6]);
    assert!(report.passed());
    assert!(report.distinct_inputs_ratio >= 0.5, "{}", report.distinct_inputs_ratio);
    // An instrumented build run with several test threads adds
    // `coverage_threads`, which says nothing about the property.
    assert!(report.health.iter().all(|h| h.kind == "coverage_threads"), "{:?}", report.health);
    report.assert_passed();
    assert!(report.to_string().starts_with("ephyra: ok, 64 tests passed"));
}

#[test]
fn false_result_is_a_shrunk_counterexample() {
    let report = check(&cfg(), gens::vec(gens::int(0..=9)), |v: Vec<i32>| v.len() < 3);
    assert_eq!(report.status, Status::Failed);
    let failure = report.failure.as_ref().unwrap();
    assert_eq!(failure.kind, FailureKind::False);
    assert!(failure.original.debug.starts_with('['));
    assert!(!failure.flaky);
    assert_eq!(failure.iteration + 1, report.iterations);
    assert_eq!(failure.counterexample().debug, "[0, 0, 0]");
    assert!(failure.shrink.is_some());
    let point = failure.replay.unwrap();
    assert_eq!(point.seed, 1);
    let text = report.to_string();
    assert!(text.contains("property returned false"), "{text}");
    assert!(text.contains("counterexample: [0, 0, 0]"), "{text}");
    assert!(text.contains(&format!("EPHYRA_REPLAY={point}")), "{text}");
}

#[test]
fn replay_point_reproduces_without_random_search() {
    let property = |v: Vec<i32>| v.len() < 3;
    let first = check(&cfg(), gens::vec(gens::int(0..=9)), property);
    let point = first.failure.as_ref().unwrap().replay.unwrap();

    let second = check(&cfg().seed(999).replay(point), gens::vec(gens::int(0..=9)), property);
    assert_eq!(second.status, Status::Failed);
    assert_eq!(second.iterations, 0, "no random search was needed");
    assert_eq!(second.failure.as_ref().unwrap().original, first.failure.as_ref().unwrap().original);
    assert!(second.replay.as_ref().unwrap().reproduced);
    assert!(second.to_string().contains("reproduced the failure"));

    let fixed = check(&cfg().replay(point), gens::vec(gens::int(0..=9)), |_: Vec<i32>| true);
    assert_eq!(fixed.status, Status::Passed);
    assert!(!fixed.replay.as_ref().unwrap().reproduced);
    assert_eq!(fixed.tests, 64);
}

#[cfg(feature = "json")]
#[test]
fn regressions_are_saved_and_replayed() {
    let dir =
        std::env::temp_dir().join(format!("ephyra-runner-regressions-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let config = Config::default().seed(5).tests(64).name("sum_prop").regressions_dir(&dir);
    let property = |v: Vec<i32>| v.iter().sum::<i32>() < 10;

    let first = check(&config, gens::vec(gens::int(0..=9)), property);
    assert_eq!(first.status, Status::Failed);
    assert!(first.regressions.saved);
    let file = first.regressions.file.clone().unwrap();
    assert!(file.ends_with("sum_prop.jsonl"));
    assert!(first.to_string().contains("remembered in"));

    let second = check(&config.clone().seed(6), gens::vec(gens::int(0..=9)), property);
    assert_eq!(second.status, Status::Failed);
    assert_eq!(second.regressions.replayed, 1);
    assert_eq!(second.iterations, 0);
    assert_eq!(second.counterexample().unwrap().debug, first.counterexample().unwrap().debug);
    assert!(!second.regressions.saved, "the same point is not stored twice");

    let fixed = check(&config, gens::vec(gens::int(0..=9)), |_: Vec<i32>| true);
    assert_eq!(fixed.status, Status::Passed);
    assert_eq!(fixed.regressions.replayed, 1);

    let changed = check(&config, gens::vec(gens::int(100..=109)), |_: Vec<i32>| true);
    assert_eq!(changed.regressions.stale, 1);
    assert!(changed.health.iter().any(|w| w.kind == "regressions"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(feature = "json")]
#[test]
fn json_report_file_is_appended() {
    let dir = std::env::temp_dir().join(format!("ephyra-runner-json-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("reports.jsonl");
    let mut config = cfg();
    config.report.json_path = Some(path.clone());
    check(&config, gens::int(0..=9), |_: i32| true);
    check(&config, gens::int(0..=9), |x: i32| x < 5);
    let text = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<serde_json::Value> =
        text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["status"], "Passed");
    assert_eq!(lines[1]["failure"]["shrunk"]["debug"], "5");
    assert!(lines[1]["failure"]["replay"]["iteration"].is_number());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn env_problems_become_warnings() {
    let (config, problems) =
        cfg().apply_env_vars(vec![("EPHYRA_BOGUS".to_string(), "1".to_string())]);
    assert_eq!(problems.len(), 1);
    let report = check(&config, gens::int(0..=9), |_: i32| true);
    assert!(report.passed());
}

#[test]
fn health_warnings_for_discards_and_degenerate_generators() {
    let report = check(&cfg(), gens::int(0..=9), |x: i32| {
        crate::assume!(x == 0);
        TestResult::Pass
    });
    assert!(report.health.iter().any(|w| w.kind == "discard_ratio"), "{:?}", report.health);
    let report = check(&cfg(), gens::constant(1), |_: i32| true);
    assert!(report.distinct_inputs_ratio < 0.1);
    assert!(report.health.iter().any(|w| w.kind == "low_distinct_inputs"), "{:?}", report.health);
}

#[test]
fn error_and_test_result_failures_carry_messages() {
    let report =
        check(
            &cfg(),
            gens::int(0..=9),
            |x: i32| {
                if x > 5 { Err(format!("too big: {x}")) } else { Ok(()) }
            },
        );
    let failure = report.failure.as_ref().unwrap();
    assert_eq!(failure.kind, FailureKind::Error);
    assert_eq!(
        failure.message.as_deref(),
        Some("\"too big: 6\""),
        "message reflects the shrunk input"
    );
    assert_eq!(failure.counterexample().debug, "6");

    let report =
        check(&cfg(), gens::int(0..=9), |x: i32| TestResult::check(x < 5, "x must be small"));
    let failure = report.failure.as_ref().unwrap();
    assert_eq!(failure.kind, FailureKind::Error);
    assert_eq!(failure.message.as_deref(), Some("x must be small"));
    assert_eq!(failure.counterexample().debug, "5");
}

#[test]
fn panics_are_captured_with_location() {
    let report = check(&cfg(), gens::int(0..=9), |x: i32| {
        if x > 3 {
            panic!("x was {x}");
        }
    });
    let failure = report.failure.as_ref().unwrap();
    assert_eq!(failure.kind, FailureKind::Panic);
    assert_eq!(failure.message.as_deref(), Some("x was 4"));
    let location = failure.location.as_ref().expect("panic location");
    assert!(location.file.ends_with("runner/tests.rs"), "{}", location.file);
    assert!(report.to_string().contains("property panicked: x was 4"));
}

#[test]
fn shrinking_keeps_the_failure_signature() {
    let report = check(&cfg().seed(3), gens::int(0..=100), |x: i32| {
        if x > 50 {
            panic!("large");
        }
        x <= 10
    });
    let failure = report.failure.as_ref().unwrap();
    match failure.kind {
        FailureKind::Panic => assert_eq!(failure.counterexample().debug, "51"),
        FailureKind::False => assert_eq!(failure.counterexample().debug, "11"),
        FailureKind::Error => unreachable!(),
    }
}

#[test]
fn attached_shrinker_is_used_and_filtered() {
    let generator =
        gens::int(0i32..=100).filter("even", |x| x % 2 == 0).with_shrink(|x: &i32, visitor| {
            visitor(x - 2)?;
            visitor(x / 2)
        });
    let report = check(&cfg(), generator, |x: i32| x < 11);
    let failure = report.failure.as_ref().unwrap();
    assert_eq!(failure.counterexample().debug, "12", "{report}");
    assert!(failure.shrink.as_ref().unwrap().steps > 0);
}

#[test]
fn shrinking_can_be_disabled() {
    let report = check(&cfg().no_shrink(), gens::vec(gens::int(0..=9)), |v: Vec<i32>| v.len() < 3);
    let failure = report.failure.as_ref().unwrap();
    assert!(failure.shrunk.is_none());
    assert!(failure.shrink.is_none());
    assert_eq!(failure.counterexample(), &failure.original);
}

#[test]
fn shrink_budget_is_reported() {
    let config = Config {
        shrink: crate::config::ShrinkConfig { max_steps: 3, ..Default::default() },
        ..cfg()
    };
    let report = check(&config, gens::vec(gens::int(0..=1000)).len(10..=20), |v: Vec<i32>| {
        v.iter().sum::<i32>() < 100
    });
    let failure = report.failure.as_ref().unwrap();
    assert!(failure.shrink.as_ref().unwrap().budget_exhausted);
    assert!(report.health.iter().any(|w| w.kind == "shrink_budget"));
    assert!(report.to_string().contains("budget exhausted"));
}

#[test]
fn discards_from_property_and_generator_are_counted() {
    let report = check(&cfg(), gens::int(0..=9), |x: i32| {
        crate::assume!(x % 2 == 0);
        TestResult::Pass
    });
    assert_eq!(report.status, Status::Passed);
    assert_eq!(report.tests, 64);
    assert!(report.discarded > 0);
    assert_eq!(
        report.discard_reasons.keys().next().map(String::as_str),
        Some("assume: x % 2 == 0")
    );

    let report = check(&cfg(), gens::int(0i32..=9).filter("even", |x| x % 2 == 0), |_: i32| true);
    assert_eq!(report.status, Status::Passed);
    assert_eq!(report.discarded, 0, "filter retries do not discard unless exhausted");
    let report = check(
        &cfg().filter_max_attempts(2),
        gens::int(0i32..=9).filter("never", |_| false),
        |_: i32| true,
    );
    assert_eq!(report.status, Status::GaveUp);
    assert_eq!(report.discard_reasons.get("filter:never").copied(), Some(report.discarded));
}

#[test]
fn gives_up_when_everything_is_discarded() {
    let report = check(&cfg(), gens::int(0..=9), |_: i32| TestResult::discard("nope"));
    assert_eq!(report.status, Status::GaveUp);
    assert_eq!(report.tests, 0);
    assert!(report.discarded > 64 * 10);
    assert!(report.to_string().starts_with("ephyra: GAVE UP"));
}

#[test]
fn times_out() {
    let config = cfg().tests(1_000_000).timeout(std::time::Duration::from_millis(20));
    let report = check(&config, gens::int(0..=9), |_: i32| {
        std::thread::sleep(std::time::Duration::from_millis(1));
        true
    });
    assert_eq!(report.status, Status::TimedOut);
    assert!(report.tests < 1_000_000);
}

#[test]
fn time_budget_stops_the_random_phase() {
    let config = cfg().tests(1_000_000).budget(std::time::Duration::from_millis(20));
    let report = check(&config, gens::int(0..=9), |_: i32| {
        std::thread::sleep(std::time::Duration::from_millis(1));
        true
    });
    assert_eq!(report.status, Status::Passed);
    assert!(report.stopped_by_budget);
    assert!(report.tests < 1_000_000);
    assert!(report.to_string().contains("stopped by the time budget"));
}

#[test]
fn generator_panic_aborts() {
    let report = check(&cfg(), |_: &mut Source| -> i32 { panic!("generator bug") }, |_: i32| true);
    assert_eq!(report.status, Status::Aborted);
    let abort = report.abort.as_ref().unwrap();
    assert_eq!(abort.message, "generator bug");
    assert!(abort.location.is_some());
    assert!(report.to_string().contains("ABORTED"));
}

#[test]
fn labels_are_reported_for_passing_tests() {
    let report = check(&cfg(), gens::vec(gens::int(0..=9)), |v: Vec<i32>| {
        classify(v.is_empty(), "empty");
        classify(!v.is_empty(), "non-empty");
        true
    });
    let empty = report.labels.get("empty").copied().unwrap_or(0);
    let non_empty = report.labels.get("non-empty").copied().unwrap_or(0);
    assert_eq!(empty + non_empty, 64);
    assert!(report.to_string().contains("labels: empty"));
}

#[test]
fn same_seed_same_report() {
    let run =
        || check(&cfg(), gens::vec(gens::int(0..=9)), |v: Vec<i32>| v.iter().sum::<i32>() < 20);
    let a = run();
    let b = run();
    assert_eq!(a.status, Status::Failed);
    let fa = a.failure.as_ref().unwrap();
    let fb = b.failure.as_ref().unwrap();
    assert_eq!(fa.original, fb.original);
    assert_eq!(fa.shrunk, fb.shrunk);
    assert_eq!(fa.replay, fb.replay);
    assert_eq!(a.tests, b.tests);
    let c = check(&cfg().seed(2), gens::vec(gens::int(0..=9)), |v: Vec<i32>| {
        v.iter().sum::<i32>() < 20
    });
    assert_ne!(a.seed, c.seed);
}

#[test]
fn unseeded_runs_get_distinct_seeds() {
    let a = check(&Config::default().tests(1).no_regressions(), gens::bool(), |_| true);
    let b = check(&Config::default().tests(1).no_regressions(), gens::bool(), |_| true);
    assert_ne!(a.seed, b.seed);
}

#[test]
fn flaky_property_is_flagged_and_not_shrunk() {
    let calls = Cell::new(0);
    let report = check(&cfg(), gens::int(0..=9), |_: i32| {
        calls.set(calls.get() + 1);
        calls.get() != 3
    });
    let failure = report.failure.as_ref().unwrap();
    assert!(failure.flaky);
    assert!(failure.shrunk.is_none());
    assert!(report.health.iter().any(|h| h.kind == "flaky"), "{:?}", report.health);
    assert!(report.to_string().contains("FLAKY"));
}

#[test]
fn debug_output_is_truncated() {
    let config = Config {
        report: crate::config::ReportConfig { truncate_debug_at: 10, ..Default::default() },
        ..cfg().no_shrink()
    };
    let report = check(&config, gens::vec(gens::int(0..=9)).len(50..=50), |_: Vec<i32>| false);
    let original = &report.failure.as_ref().unwrap().original;
    assert!(original.truncated);
    assert_eq!(original.debug.chars().count(), 11);
    assert!(report.to_string().contains("(truncated)"));
}

#[test]
fn concurrent_checks_keep_their_own_panics() {
    let handles: Vec<_> = (0..6)
        .map(|i| {
            std::thread::spawn(move || {
                let report = check(&cfg(), gens::int(0..=9), move |x: i32| {
                    if x > 2 {
                        panic!("worker {i}");
                    }
                });
                let failure = report.failure.unwrap();
                assert_eq!(failure.message.as_deref(), Some(format!("worker {i}").as_str()));
                assert_eq!(failure.counterexample().debug, "3");
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }
}

#[test]
fn targeted_search_finds_what_random_search_misses() {
    let generator = || gens::vec(gens::int(0u8..=255)).len(8..=8);
    let property = |v: Vec<u8>| {
        let sum: u32 = v.iter().map(|&x| u32::from(x)).sum();
        crate::target(f64::from(sum));
        sum < 1900
    };
    let mut random_only = cfg().seed(11);
    random_only.target.tests = 0;
    let report = check(&random_only, generator(), property);
    assert_eq!(report.status, Status::Passed, "{report}");
    let summary = report.target.as_ref().expect("scores were observed");
    assert_eq!(summary.iterations, 0);
    assert!(summary.best_score < 1900.0);

    let mut targeted = cfg().seed(11);
    targeted.target.tests = 2000;
    let report = check(&targeted, generator(), property);
    assert_eq!(report.status, Status::Failed, "{report}");
    let failure = report.failure.as_ref().unwrap();
    assert!(failure.replay.is_none(), "mutants replay through the run seed");
    let shrunk: Vec<u32> = failure
        .counterexample()
        .debug
        .trim_matches(|c| c == '[' || c == ']')
        .split(", ")
        .map(|x| x.parse().unwrap())
        .collect();
    assert_eq!(shrunk.iter().sum::<u32>(), 1900, "{shrunk:?}");
    assert!(report.to_string().contains("found in the targeted phase"));

    let again = check(&targeted, generator(), property);
    assert_eq!(again.failure.as_ref().unwrap().original, failure.original, "deterministic");
}

#[test]
fn target_summary_reports_the_best_input() {
    let generator = gens::vec(gens::int(0..=9)).len(4..=4);
    let property = |v: Vec<i32>| {
        crate::target(f64::from(v.iter().sum::<i32>()));
        true
    };
    let mut random_only = cfg().seed(4);
    random_only.target.tests = 0;
    let baseline = check(&random_only, generator.clone(), property);
    let baseline_best = baseline.target.as_ref().unwrap().best_score;

    let mut config = cfg().seed(4);
    config.target.tests = 300;
    let report = check(&config, generator.clone(), property);
    assert!(report.passed());
    let summary = report.target.as_ref().unwrap();
    assert!(summary.best_score > baseline_best, "{} vs {baseline_best}", summary.best_score);
    assert!(summary.best_score >= 30.0, "{}", summary.best_score);
    assert_eq!(summary.best_debug.matches(',').count(), 3, "{}", summary.best_debug);
    assert_eq!(summary.iterations, 300);
    assert!(summary.improvements > 0);
    assert!(report.to_string().contains("target: best score"));

    let plain = check(&config, generator, |_: Vec<i32>| true);
    assert!(plain.target.is_none(), "no scores, no targeted phase");
    assert_eq!(plain.iterations, 64);
}

#[test]
fn features_timing_and_samples_are_reported() {
    let mut config = cfg();
    config.tests = 200;
    let report = check(&config, gens::vec(gens::int(0i32..=9)), |v: Vec<i32>| {
        crate::feature("len", v.len());
        crate::feature("parity", if v.len().is_multiple_of(2) { "even" } else { "odd" });
        crate::target(v.iter().sum::<i32>() as f64);
        true
    });
    assert_eq!(report.status, Status::Passed, "{report}");
    let len = &report.features["len"];
    assert_eq!(len.count, report.tests, "the targeted phase's passes count too");
    assert!(len.min <= len.median && len.median <= len.p90 && len.p90 <= len.max);
    assert_eq!(len.histogram.iter().map(|b| b.count).sum::<u64>(), report.tests, "{len:?}");
    assert!(report.features.contains_key("target"));
    assert_eq!(
        report.labels.get("parity=even").copied().unwrap_or(0)
            + report.labels.get("parity=odd").copied().unwrap_or(0),
        report.tests
    );
    assert!(report.unique_tests <= report.tests && report.unique_tests > 100, "{report}");
    assert!(!report.distinct_growth.points.is_empty());
    let timing = report.timing.as_ref().expect("inputs ran");
    assert!(timing.prop.median <= timing.prop.p90 && timing.prop.p90 <= timing.prop.max);
    assert!(timing.slowest.is_some());
    assert!(!report.samples.is_empty() && report.samples.len() <= 5, "{report}");
    let smallest = &report.samples[0];
    assert!(report.samples.iter().all(|s| s.size >= smallest.size));
    let text = report.to_string();
    assert!(text.contains("feature len:") && text.contains("histogram:"), "{text}");
    assert!(text.contains("timing: property median") && text.contains("sample ("), "{text}");
    assert!(text.contains("distinct,"), "{text}");
}

#[cfg(feature = "json")]
#[test]
fn observation_files_follow_openpbtstats() {
    let dir = std::env::temp_dir().join(format!("ephyra-observed-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut config = cfg().name("observed");
    config.tests = 50;
    config.report.observations = Some(dir.clone());
    let generator = (|source: &mut Source| vec![9; source.size().min(5)]).with_shrink(
        |value: &Vec<i32>, visitor| {
            if value.iter().any(|&x| x != 1) {
                visitor(vec![1; value.len()])?;
            }
            std::ops::ControlFlow::Continue(())
        },
    );
    let report = check(&config, generator, |v: Vec<i32>| {
        crate::feature("len", v.len());
        crate::observe("elements", || v.len());
        crate::label("seen");
        if v.len() == 4 {
            crate::TestResult::discard("four")
        } else {
            crate::TestResult::check(v.len() < 5, "too long")
        }
    });
    assert_eq!(report.status, Status::Failed, "{report}");
    let text = std::fs::read_to_string(dir.join("observed.jsonl")).expect("file written");
    let _ = std::fs::remove_dir_all(&dir);
    let lines: Vec<serde_json::Value> =
        text.lines().map(|l| serde_json::from_str(l).expect("valid json")).collect();
    let info = lines.last().unwrap();
    assert_eq!(info["type"], "info");
    assert_eq!(info["title"], "ephyra report");
    assert!(info["content"].as_str().unwrap().contains("ephyra: FAILED"));
    let cases: Vec<&serde_json::Value> =
        lines.iter().filter(|l| l["type"] == "test_case").collect();
    assert!(
        cases.iter().all(|c| c["run_start"] == info["run_start"] && c["property"] == "observed")
    );
    let count = |status: &str, origin: &str| {
        cases.iter().filter(|c| c["status"] == status && c["how_generated"] == origin).count()
            as u64
    };
    assert_eq!(count("passed", "random"), report.tests);
    assert_eq!(count("gave_up", "random"), report.discarded);
    assert_eq!(count("failed", "random"), 1);
    assert!(count("passed", "shrink") + count("failed", "shrink") > 0, "shrink candidates");
    let shrink_cases: Vec<_> =
        cases.iter().filter(|case| case["how_generated"] == "shrink").collect();
    assert!(shrink_cases.iter().all(|case| case["metadata"]["shrink_accepted"].is_boolean()));
    assert!(
        shrink_cases.iter().any(|case| case["metadata"]["shrink_accepted"] == true),
        "at least one shrink candidate should be accepted"
    );
    let last_case = cases.last().unwrap();
    assert_eq!(last_case["how_generated"], "minimal");
    assert_eq!(last_case["status"], "failed");
    assert_eq!(
        last_case["representation"],
        report.failure.as_ref().unwrap().counterexample().debug
    );
    let passed = cases.iter().find(|c| c["status"] == "passed").unwrap();
    assert!(passed["representation"].as_str().unwrap().starts_with('['));
    assert!(passed["features"]["len"].is_number());
    assert_eq!(passed["features"]["seen"], "");
    assert!(passed["observations"]["elements"].is_number());
    assert!(passed["timing"]["execute:test"].is_number());
    assert!(passed["timing"]["generate:input"].is_number());
    assert_eq!(passed["metadata"]["seed"], report.seed);
    let failed = cases.iter().find(|c| c["status"] == "failed").unwrap();
    assert_eq!(failed["status_reason"], "too long");
    let discarded = cases.iter().find(|c| c["status"] == "gave_up").unwrap();
    assert_eq!(discarded["status_reason"], "four");
    #[cfg(feature = "coverage")]
    if crate::coverage::available() && crate::coverage::locate::Spectrum::new(&[]).is_some() {
        assert!(passed["coverage"].is_object(), "{passed}");
    } else {
        assert!(passed["coverage"].is_null(), "{passed}");
    }
    #[cfg(not(feature = "coverage"))]
    assert!(passed["coverage"].is_null(), "{passed}");
}

#[test]
fn installed_sinks_see_the_runs_they_open() {
    use std::sync::{
        Arc,
        Mutex,
    };

    use crate::sink::{
        CaseStatus,
        RunInfo,
        RunSink,
        Sink,
    };

    #[derive(Default)]
    struct Seen {
        opened: Vec<(String, u64)>,
        cases: Vec<(CaseStatus, String, String, bool)>,
        finished: Option<Status>,
    }
    struct Recorder(Arc<Mutex<Seen>>);
    struct Recording(Arc<Mutex<Seen>>);
    impl Sink for Recorder {
        fn open(&self, run: &RunInfo<'_>) -> Option<Box<dyn RunSink>> {
            if !run.property.starts_with("sink-test") {
                return None;
            }
            self.0.lock().unwrap().opened.push((run.property.to_string(), run.seed));
            Some(Box::new(Recording(self.0.clone())))
        }
    }
    impl RunSink for Recording {
        fn test_case(&mut self, case: &crate::sink::TestCase<'_>) {
            let observed = case.observations.iter().any(|(name, _)| name == "elements");
            self.0.lock().unwrap().cases.push((
                case.status,
                case.status_reason.to_string(),
                case.origin.to_string(),
                observed,
            ));
        }
        fn finish(&mut self, report: &Report) -> Result<(), String> {
            self.0.lock().unwrap().finished = Some(report.status);
            Err("the agent is away".to_string())
        }
    }

    let seen = Arc::new(Mutex::new(Seen::default()));
    crate::sink::install(Recorder(seen.clone()));
    let mut config = cfg().name("sink-test-failing");
    config.tests = 100;
    let report = check(&config, gens::int(0i32..=100), |n: i32| {
        crate::observe("elements", || n);
        crate::TestResult::check(n < 50, "too big")
    });
    assert_eq!(report.status, Status::Failed, "{report}");
    assert!(
        report.health.iter().any(|w| w.kind == "observations" && w.message == "the agent is away"),
        "{report}"
    );
    let unseen = check(&cfg().name("other").tests(10), gens::int(0i32..=9), |_| true);
    assert!(unseen.passed());

    let seen = seen.lock().unwrap();
    assert_eq!(seen.opened, vec![("sink-test-failing".to_string(), report.seed)]);
    assert_eq!(seen.finished, Some(Status::Failed));
    let passes = seen.cases.iter().filter(|c| c.0 == CaseStatus::Passed && c.2 == "random");
    assert_eq!(passes.count() as u64, report.tests);
    assert!(seen.cases.iter().all(|c| c.3), "observe closures run while a sink is open");
    let last = seen.cases.last().unwrap();
    assert_eq!(
        (last.0, last.1.as_str(), last.2.as_str()),
        (CaseStatus::Failed, "too big", "minimal")
    );
}

#[test]
fn samples_keep_the_smallest_the_largest_and_a_reservoir() {
    let mut samples = Samples::new(5);
    for (i, size) in [5usize, 3, 9, 1, 7, 8, 2, 6].into_iter().enumerate() {
        let r = hash_u01(1, i as u64);
        samples.consider(size, i as u64, crate::report::Origin::Random, r, || {
            Provenance::Debug(size.to_string())
        });
    }
    let kept = samples.drain();
    assert_eq!(kept[0].size, 1);
    assert_eq!(kept[1].size, 9);
    assert!(kept.len() <= 5 && kept.len() >= 3, "{}", kept.len());
    let mut none = Samples::new(0);
    none.consider(1, 0, crate::report::Origin::Random, 0.5, || Provenance::Debug(String::new()));
    assert!(none.drain().is_empty());
}

#[test]
fn cover_labels_are_reported_and_warned() {
    let report = check(&cfg(), gens::vec(gens::int(0..=9)), |v: Vec<i32>| {
        crate::cover(v.len() > 100, "long");
        crate::cover(true, "always");
        true
    });
    assert_eq!(report.covers.get("long"), Some(&0.0));
    assert_eq!(report.covers.get("always"), Some(&1.0));
    let warning = report.health.iter().find(|w| w.kind == "cover").expect("cover warning");
    assert!(warning.message.contains("`long`"), "{}", warning.message);
    assert!(report.to_string().contains("cover: always 100%, long 0%"), "{report}");
}

#[cfg(feature = "coverage")]
#[test]
fn the_default_coverage_filter_is_the_property_crate() {
    assert_eq!(crate_of("bst::main::{{closure}}"), "bst");
    assert_eq!(crate_of("&my_crate::props::sorted"), "my_crate");
    assert_eq!(crate_of("&mut my_crate::props::sorted"), "my_crate");
    assert_eq!(crate_of("ephyra"), "ephyra");
    assert_eq!(crate_of(crate_of_probe_name()), "ephyra");
}

#[cfg(all(ephyra_instrumented, feature = "coverage"))]
const BRANCHY_LINE: u32 = line!() + 3;
#[cfg(all(ephyra_instrumented, feature = "coverage"))]
#[rustfmt::skip]
fn branchy(x: i32) -> i32 {
    if x > 1000 {
        1
    } else {
        2
    }
}

#[cfg(all(ephyra_instrumented, feature = "coverage"))]
#[test]
fn coverage_targets_are_resolved_tracked_and_reported() {
    use crate::config::CoverageTarget;
    let then_branch = CoverageTarget::new(file!(), BRANCHY_LINE + 2..=BRANCHY_LINE + 2);
    let else_branch = CoverageTarget::new(file!(), BRANCHY_LINE + 4..=BRANCHY_LINE + 4);
    let nowhere = CoverageTarget::new("no/such/file.rs", 1..=1);
    let mut config = cfg();
    config.tests = 100;
    config.coverage.targets = vec![then_branch.clone(), else_branch.clone(), nowhere.clone()];
    config.coverage.targets_file = None;
    let report = check(&config, gens::int(-100i32..=100), |x| branchy(x) == 2);
    assert_eq!(report.status, Status::Passed, "{report}");
    let coverage = report.coverage.as_ref().expect("instrumented");
    assert_eq!(coverage.targets.len(), 3, "{report}");
    let (then, els, none) = (&coverage.targets[0], &coverage.targets[1], &coverage.targets[2]);
    assert_eq!(then.target, then_branch);
    assert!(then.functions.iter().any(|f| f.contains("branchy")), "{then:?}");
    assert_eq!(then.hits, 0, "{then:?}");
    assert!(then.function_inputs > 0, "{then:?}");
    assert_eq!(els.hits, els.function_inputs, "{els:?}");
    assert_eq!(els.first_hit_at, Some(1), "{els:?}");
    assert!(none.functions.is_empty() && none.target == nowhere, "{none:?}");
    let kinds: Vec<&str> = report.health.iter().map(|w| w.kind.as_str()).collect();
    assert!(kinds.contains(&"coverage_target_missed"), "{kinds:?}");
    assert!(kinds.contains(&"coverage_target"), "{kinds:?}");
    let text = report.to_string();
    assert!(text.contains("never reached") && text.contains("took other branches"), "{text}");
    assert!(text.contains("hit by"), "{text}");
    assert!(report.target.is_some(), "targets turn the guided search on: {report}");
}

#[cfg(all(ephyra_instrumented, feature = "coverage"))]
#[test]
fn novelty_counts_what_an_input_reached_first() {
    let mut config = cfg();
    config.seed = Some(1);
    config.coverage.enabled = true;
    let mut run = Run::new(&config, gens::int(0i32..=1), |x: i32| branchy(x) > 0);
    let input = run.draw().expect("ints never discard");
    let first = run.test(&input);
    assert!(first.novel() && first.novelty() > 0, "{first:?}");
    let again = run.test(&input);
    assert!(!again.novel(), "{again:?}");
    assert_eq!(again.novelty(), 0, "the same input reaches nothing new");
    let other = Input { value: 2000, ..input.clone() };
    let differs = run.test(&other);
    assert!(differs.novelty() > 0, "the other branch is new: {differs:?}");
    run.finish().assert_passed();
}

#[cfg(all(ephyra_instrumented, feature = "coverage"))]
#[test]
fn the_coverage_grid_is_written() {
    use crate::config::CoverageTarget;
    let dir = std::env::temp_dir().join(format!("ephyra-grid-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut config = cfg().name("grid::branchy");
    config.tests = 50;
    config.coverage.html = Some(dir.clone());
    config.coverage.targets =
        vec![CoverageTarget::new(file!(), BRANCHY_LINE + 2..=BRANCHY_LINE + 2)];
    config.coverage.targets_file = None;
    let report = check(&config, gens::int(-100i32..=100), |x| branchy(x) == 2);
    let page = report.coverage_html.clone().expect("grid written");
    assert_eq!(page, dir.join("grid_branchy.html"));
    assert!(report.to_string().contains("coverage grid written to"), "{report}");
    let html = std::fs::read_to_string(&page).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(html.contains("ephyra::runner::tests::branchy"), "function listed");
    assert!(html.contains("\"file_index\""), "targets embedded");
    assert!(html.contains("fn branchy(x: i32) -> i32 {"), "source embedded");
    // The page embeds this very file as source, so look at the script
    // tag rather than for the placeholder's absence.
    assert!(html.contains("<script id=\"data\" type=\"application/json\">{"), "data embedded");
    let start = html.find("<script id=\"data\"").unwrap();
    let end = html[start..].find("</script>").unwrap() + start;
    let json_text = &html[html[start..].find('>').unwrap() + start + 1..end];
    let data: serde_json::Value = serde_json::from_str(json_text).expect("embedded JSON parses");
    assert_eq!(data["property"], "grid::branchy");
    let property_fn = data["property_fn"].as_str().unwrap_or_default();
    assert!(
        property_fn.starts_with("ephyra::runner::tests::the_coverage_grid_is_written::{closure#"),
        "{property_fn}"
    );
    assert!(
        data["files"].as_array().unwrap().iter().any(|f| {
            f["functions"].as_array().unwrap().iter().any(|func| {
                func["name"] == "ephyra::runner::tests::branchy"
                    && func["lines"].as_array().unwrap().iter().any(|l| l[1] == 0)
                    && func["lines"].as_array().unwrap().iter().any(|l| l[1].as_u64().unwrap() > 0)
            })
        }),
        "{data}"
    );
    assert_eq!(data["targets"][0]["hits"], 0);
}

#[cfg(feature = "coverage")]
#[test]
fn coverage_summary_matches_the_build() {
    let report = check(&cfg(), gens::vec(gens::int(0..=9)), |v: Vec<i32>| v.len() <= 100);
    if crate::coverage::available() {
        let coverage = report.coverage.as_ref().expect("coverage collected");
        assert!(coverage.functions_total > 0);
        assert_eq!(coverage.inputs, 64);
        assert!(report.to_string().contains("coverage:"), "{report}");
    } else {
        assert!(report.coverage.is_none());
        assert!(
            !report.health.iter().any(|w| w.kind.starts_with("coverage")),
            "{:?}",
            report.health
        );
    }
    let mut wants = cfg();
    wants.coverage.guided = true;
    let report = check(&wants, gens::vec(gens::int(0..=9)), |v: Vec<i32>| v.len() <= 100);
    if crate::coverage::available() {
        assert!(report.target.is_some(), "guided search seeds the pool from novelty: {report}");
    } else {
        assert!(
            report.health.iter().any(|w| w.kind == "coverage_unavailable"),
            "{:?}",
            report.health
        );
    }
}

#[test]
fn hand_written_loops_replay_by_seed() {
    fn climb(seed: u64) -> Report {
        let mut config = cfg();
        config.seed = Some(seed);
        config.tests = 100;
        config.max_size = 20;
        let mut run = Run::new(&config, gens::vec(gens::int(0i32..=9)), |v: Vec<i32>| {
            let sum: i32 = v.iter().sum();
            crate::target(f64::from(sum));
            sum < 500
        });
        while run.wants_more() {
            let Some(input) = run.draw() else { continue };
            let tested = run.test(&input);
            if tested.passed() {
                run.keep(input.value, &tested);
            }
        }
        for _ in 0..200 {
            if run.report().status != Status::Passed {
                break;
            }
            let Some(parent) = run.pool_pick() else { break };
            let Some(child) = run.mutate(&parent) else { continue };
            let tested = run.test(&child);
            if tested.failed() {
                break;
            }
            run.keep(child.value, &tested);
        }
        run.finish()
    }
    let a = climb(11);
    let b = climb(11);
    assert_eq!(a.status, Status::Passed, "{a}");
    assert_eq!((a.tests, a.iterations), (b.tests, b.iterations));
    assert_eq!(a.iterations, 300, "100 draws and 200 mutations: {a}");
    assert_eq!(a.tests + a.discarded, a.iterations, "passing mutants count as tests");
    let target = a.target.as_ref().expect("mutations are summarised");
    assert_eq!(target.iterations, 200);
    assert_eq!(target.best_score, b.target.as_ref().unwrap().best_score);
    assert_ne!(climb(12).seed, a.seed);
}

#[test]
fn a_failure_found_by_mutation_is_reported_without_a_replay_point() {
    let mut config = cfg();
    config.seed = Some(5);
    config.max_size = 8;
    let mut run = Run::new(&config, gens::vec(gens::int(0i32..=9)), |v: Vec<i32>| v.is_empty());
    let input = loop {
        if let Some(input) = run.draw()
            && !input.value.is_empty()
        {
            break input;
        }
    };
    assert!(input.replay().is_some(), "drawn inputs regenerate from a point");
    let mutant = loop {
        if let Some(mutant) = run.mutate(&input.value)
            && !mutant.value.is_empty()
        {
            break mutant;
        }
    };
    assert!(mutant.replay().is_none(), "mutants do not");
    assert_eq!(mutant.origin(), crate::report::Origin::Mutation);
    let tested = run.test(&mutant);
    assert!(tested.failed());
    assert!(!run.wants_more(), "a failure stops the loop");
    let report = run.finish();
    assert_eq!(report.status, Status::Failed, "{report}");
    let failure = report.failure.expect("failure");
    assert!(failure.replay.is_none(), "found by mutation: {failure:?}");
    assert_eq!(failure.iteration, mutant.iteration());
    assert_eq!(report.tests, 0, "nothing passed");
}

#[test]
fn loops_can_shrink_on_their_own_terms() {
    fn failing_run() -> Run<impl Gen<Value = Vec<i32>>, impl FnMut(Vec<i32>) -> bool> {
        let mut config = cfg();
        config.seed = Some(3);
        config.max_size = 30;
        let mut run =
            Run::new(&config, gens::vec(gens::int(0i32..=9)).len(..=30), |v: Vec<i32>| {
                v.iter().sum::<i32>() < 20
            });
        while run.wants_more() {
            let Some(input) = run.draw() else { continue };
            run.test(&input);
        }
        assert!(run.failure().is_some(), "{}", run.report());
        run
    }

    // The built-in driver, on demand.
    let mut run = failing_run();
    let original = run.failure().unwrap().clone();
    assert!(run.reproduces(&original), "the original reproduces");
    assert!(!run.reproduces(&vec![]), "an empty list passes");
    let minimal = run.shrink().unwrap().clone();
    let sum: i32 = minimal.iter().sum();
    assert!(
        sum >= 20 && sum <= original.iter().sum() && minimal.len() <= original.len(),
        "{minimal:?}"
    );
    let report = run.finish();
    let failure = report.failure.expect("failed");
    assert_eq!(failure.shrunk.as_ref().map(|c| c.debug.clone()), Some(format!("{minimal:?}")));
    assert!(failure.shrink.unwrap().steps >= 3, "the loop's steps count too");

    // A greedy shrinker written by hand: drop one element at a time.
    let mut run = failing_run();
    let mut current = run.failure().unwrap().clone();
    'outer: loop {
        for i in 0..current.len() {
            let mut candidate = current.clone();
            candidate.remove(i);
            if run.reproduces(&candidate) {
                current = candidate;
                continue 'outer;
            }
        }
        break;
    }
    assert!(run.shrunk_to(current.clone()));
    let report = run.finish();
    let failure = report.failure.as_ref().expect("failed");
    assert_eq!(failure.shrunk.as_ref().map(|c| c.debug.clone()), Some(format!("{current:?}")));
    assert!(current.iter().sum::<i32>() >= 20 && current.iter().sum::<i32>() < 30, "{current:?}");
    assert!(!report.health.iter().any(|w| w.kind == "shrunk_to"), "{report}");

    // A wrong claim is refused.
    let mut run = failing_run();
    assert!(!run.shrunk_to(vec![1]));
    let report = run.finish();
    assert!(report.health.iter().any(|w| w.kind == "shrunk_to"), "{report}");
    assert!(report.failure.unwrap().shrunk.is_none(), "the original stands");
}

#[cfg(feature = "json")]
#[test]
fn report_serialises_to_json() {
    let report = check(&cfg(), gens::int(0..=9), |x: i32| x < 5);
    let json = report.to_json();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["status"], "Failed");
    assert_eq!(value["seed"], 1);
    assert_eq!(value["failure"]["shrunk"]["debug"], "5");
    assert_eq!(value["failure"]["replay"]["seed"], 1);
    assert!(value["failure"]["shrink"]["steps"].is_number());
}
