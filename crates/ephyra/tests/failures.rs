// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Several distinct failures in one run: `Config::failures`, `signature`
//! and `Config::signature`, through `check`, `#[ephyra::test]` and loops
//! written by hand.

use ephyra::{
    Config,
    Report,
    Run,
    Status,
    TestResult,
    check,
    gens,
    report::FailureKind,
    signature,
};

fn cfg() -> Config {
    Config::default().seed(1).tests(500).no_regressions()
}

/// Two panic sites and a `false`.
fn three_bugs(v: Vec<u8>) -> bool {
    assert!(!v.contains(&3), "a three in {v:?}");
    assert!(!v.contains(&7), "a seven in {v:?}");
    v.len() < 6
}

fn digits() -> impl ephyra::Gen<Value = Vec<u8>> {
    gens::vec(gens::int(0u8..=9))
}

fn signatures(report: &Report) -> Vec<&str> {
    report.failures.iter().map(|f| f.signature.as_str()).collect()
}

fn counterexamples(report: &Report) -> Vec<&str> {
    report.failures.iter().map(|f| f.failure.counterexample().debug.as_str()).collect()
}

#[test]
fn one_failure_is_the_default_and_unchanged() {
    assert_eq!(Config::default().failures, 1);
    let default = check(&cfg(), digits(), three_bugs);
    let one = check(&cfg().failures(1), digits(), three_bugs);
    assert_eq!(default.status, Status::Failed);
    assert!(default.failures.is_empty());
    assert_eq!(default.iterations, one.iterations);
    let (a, b) = (default.failure.as_ref().unwrap(), one.failure.as_ref().unwrap());
    assert_eq!((&a.original, &a.shrunk, &a.message), (&b.original, &b.shrunk, &b.message));
    assert_eq!(
        default.failure.as_ref().unwrap().iteration + 1,
        default.iterations,
        "the run stops at the first failure"
    );
    assert!(!default.to_string().contains("failure 1 of"));
    #[cfg(feature = "json")]
    assert!(!default.to_json().contains("\"failures\": "));
}

#[test]
fn several_distinct_failures_are_found_in_one_run() {
    let report = check(&cfg().failures(10), digits(), three_bugs);
    assert_eq!(report.status, Status::Failed);
    assert_eq!(report.failures.len(), 3, "{report}");
    assert_eq!(report.tests, 500, "the run went on to its budget");
    let mut found = counterexamples(&report);
    found.sort();
    assert_eq!(found, ["[0, 0, 0, 0, 0, 0]", "[3]", "[7]"]);
    // Panics are named by their location, `false` by a fixed name.
    assert!(signatures(&report).contains(&"returned false"));
    for found in &report.failures {
        match found.failure.kind {
            FailureKind::Panic => {
                let location = found.failure.location.as_ref().unwrap();
                assert_eq!(found.signature, location.to_string());
                assert!(found.signature.contains("tests/failures.rs:"), "{}", found.signature);
            },
            FailureKind::False => assert_eq!(found.signature, "returned false"),
            FailureKind::Error => unreachable!(),
        }
        assert!(found.count >= 1);
        assert!(found.failure.replay.is_some());
    }
    // Many inputs hold a 3; all of them count against one failure.
    let threes = report.failures.iter().find(|f| f.failure.counterexample().debug == "[3]");
    assert!(threes.unwrap().count > 10, "{report}");
    assert_eq!(report.failure.as_ref(), Some(&report.failures[0].failure));
}

#[test]
fn the_run_stops_once_enough_failures_are_found() {
    let report = check(&cfg().failures(2), digits(), three_bugs);
    assert_eq!(report.failures.len(), 2);
    assert!(report.tests < 500, "{report}");
}

#[test]
fn signature_names_the_failures() {
    let report = check(&cfg().failures(10), gens::int(0u32..=1000), |x| {
        if x >= 100 {
            signature(if x % 2 == 0 { "big even" } else { "big odd" });
            // One panic site, two signatures.
            panic!("too big: {x}");
        }
    });
    let mut names = signatures(&report);
    names.sort();
    assert_eq!(names, ["big even", "big odd"]);
    // Shrinking kept each signature, though both fail at the same site:
    // the odd failure did not shrink to an even number.
    for found in &report.failures {
        let shrunk: u32 = found.failure.counterexample().debug.parse().unwrap();
        assert!(found.failure.shrunk.is_some(), "{report}");
        assert!(shrunk >= 100, "{report}");
        assert_eq!(shrunk.is_multiple_of(2), found.signature == "big even", "{report}");
        assert_eq!(found.failure.message, Some(format!("too big: {shrunk}")));
    }
}

#[test]
fn shrinking_keeps_the_default_signature() {
    let report = check(&cfg().failures(10), gens::int(0u32..=1000), |x| {
        if x >= 500 {
            assert!(x % 2 == 0, "large and odd");
        }
        assert!(x < 100, "not small");
    });
    assert_eq!(report.failures.len(), 2, "{report}");
    for found in &report.failures {
        let shrunk: u32 = found.failure.counterexample().debug.parse().unwrap();
        let large_odd = shrunk >= 500 && shrunk % 2 == 1;
        assert_eq!(large_odd, found.failure.message.as_deref() == Some("large and odd"));
    }
}

#[test]
fn error_messages_are_grouped_without_their_values() {
    let report = check(&cfg().failures(10), gens::vec(gens::int(0u32..=99)), |v| {
        if v.len() > 3 {
            return TestResult::fail(format!("too long: {} items, first {:?}", v.len(), v[0]));
        }
        TestResult::Pass
    });
    assert_eq!(signatures(&report), ["too long: N items, first N"], "{report}");
    assert_eq!(counterexamples(&report), ["[0, 0, 0, 0]"]);
    assert!(report.failures[0].count > 1);
}

#[test]
fn a_config_signature_groups_failures() {
    let config = cfg().failures(10).signature(|failure| {
        assert_eq!(failure.kind, FailureKind::Error);
        let value: u32 = failure.value.parse().unwrap();
        format!("{} digits", value.to_string().len())
    });
    let report = check(&config, gens::int(0u32..=1000), |x| {
        if x % 2 == 1 { Err(format!("odd {x}")) } else { Ok(()) }
    });
    let mut names = signatures(&report);
    names.sort();
    assert_eq!(names, ["1 digits", "2 digits", "3 digits"]);
    for found in &report.failures {
        let shrunk = &found.failure.counterexample().debug;
        assert_eq!(format!("{} digits", shrunk.len()), found.signature, "{report}");
    }
}

#[test]
fn signature_wins_over_the_config_function() {
    let config = cfg().failures(10).signature(|_| "from the config".to_string());
    let report = check(&config, gens::int(0u32..=1000), |x| {
        if x % 3 == 0 {
            signature("from the property");
        }
        x % 2 == 0 || x % 3 == 0 && x > 500
    });
    let mut names = signatures(&report);
    names.sort();
    assert_eq!(names, ["from the config", "from the property"], "{report}");
}

#[test]
fn each_failure_replays_from_its_point() {
    let report = check(&cfg().failures(10), digits(), three_bugs);
    assert_eq!(report.failures.len(), 3);
    for found in &report.failures {
        let point = found.failure.replay.expect("found in the random phase");
        let replayed = check(&cfg().seed(99).failures(10).replay(point), digits(), three_bugs);
        assert!(replayed.replay.as_ref().unwrap().reproduced);
        assert_eq!(replayed.iterations, 0, "no search after a failing replay point");
        assert_eq!(signatures(&replayed), [found.signature.as_str()]);
        assert_eq!(replayed.failures[0].failure.original, found.failure.original);
        // And with the default of one failure.
        let one = check(&cfg().seed(99).replay(point), digits(), three_bugs);
        assert_eq!(one.failure.as_ref().unwrap().original, found.failure.original);
    }
}

#[cfg(feature = "json")]
#[test]
fn each_failure_is_remembered_and_replayed() {
    let dir = std::env::temp_dir().join(format!("ephyra-failures-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let config =
        Config::default().seed(1).tests(500).failures(10).name("three_bugs").regressions_dir(&dir);
    let first = check(&config, digits(), three_bugs);
    assert_eq!(first.failures.len(), 3);
    assert!(first.regressions.saved);
    let stored = ephyra::regressions::load(&dir, "three_bugs");
    assert_eq!(stored.len(), 3);

    // Another seed: the stored inputs fail first, one per signature.
    let second = check(&config.clone().seed(2).failures(3), digits(), three_bugs);
    assert_eq!(second.regressions.replayed, 3);
    assert_eq!(second.failures.len(), 3);
    assert_eq!(second.iterations, 0, "three regressions are all the run wanted");
    let mut again = signatures(&second);
    again.sort();
    let mut before = signatures(&first);
    before.sort();
    assert_eq!(again, before);
    assert!(!second.regressions.saved, "nothing new to store");

    // Asking for more: the regressions are kept and the search goes on.
    let third = check(&config.clone().seed(3), digits(), three_bugs);
    assert_eq!(third.regressions.replayed, 3);
    assert_eq!(third.failures.len(), 3);
    assert!(third.tests > 0);

    // One failure: the first regression ends the run, as before.
    let one = check(&config.clone().seed(4).failures(1), digits(), three_bugs);
    assert_eq!(one.regressions.replayed, 1);
    assert!(one.failures.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn assert_passed_fails_on_any_failure() {
    let report = check(&cfg().failures(10), digits(), three_bugs);
    let text = report.to_string();
    assert!(text.contains("failure 1 of 3"), "{text}");
    assert!(text.contains("failure 3 of 3"), "{text}");
    assert!(text.contains("    reproduce: EPHYRA_REPLAY=1:"), "{text}");
    let panicked = std::panic::catch_unwind(|| report.assert_passed());
    assert!(panicked.is_err());
}

#[test]
fn hand_written_loops_collect_failures_and_may_shrink_them() {
    let config = cfg().failures(10);
    let mut run = Run::new(&config, digits(), three_bugs);
    let mut kept = 0;
    while run.wants_more() {
        let Some(input) = run.draw() else { continue };
        if run.test(&input).failed() && run.failure().is_some() {
            kept += 1;
            // The loop's own shrinking: keep the threes and sevens only.
            let only: Vec<u8> =
                run.failure().unwrap().iter().copied().filter(|&d| d == 3 || d == 7).collect();
            run.shrunk_to(only);
        }
    }
    let report = run.finish();
    assert_eq!(kept, 3);
    assert_eq!(report.failures.len(), 3, "{report}");
    for found in &report.failures {
        let best = &found.failure.counterexample().debug;
        match found.failure.kind {
            FailureKind::Panic => assert!(!best.contains('0'), "{report}"),
            // Too long without a 3 or a 7: the loop's candidate passes, and
            // that one stays as found.
            _ => assert!(found.failure.shrunk.is_none(), "{report}"),
        }
    }
    assert!(report.health.iter().any(|w| w.kind == "shrunk_to"), "{report}");
    assert_eq!(report.failure.as_ref(), Some(&report.failures[0].failure));
}

#[test]
fn shipped_search_loops_go_on_past_a_failure() {
    let config = cfg().failures(10);
    let mut run = Run::new(&config, digits(), three_bugs);
    if !run.remembered() {
        ephyra::loops::fuzz(&mut run, &mut ephyra::loops::Fifo::new(), 500);
    }
    let report = run.finish();
    assert_eq!(report.failures.len(), 3, "{report}");
}

#[cfg(feature = "derive")]
#[ephyra::test(config = Config::default().no_regressions(), failures = 5, seed = 1, tests = 500)]
#[should_panic(expected = "failure 2 of 2")]
fn the_test_attribute_collects_failures(x: u8) {
    assert!(x != 3, "three");
    assert!(x != 7, "seven");
}
