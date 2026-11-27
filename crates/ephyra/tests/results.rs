// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Properties that return a `Result` take the verdict of its `Ok` value.

use std::thread;

use ephyra::{
    Config,
    Report,
    Status,
    TestResult,
    Testable,
    check,
    gens,
};

fn run<R: Testable>(property: impl FnMut(u32) -> R) -> Report {
    let config = Config::default().seed(1).tests(100).no_regressions();
    check(&config, gens::int(0u32..=1000), property)
}

#[test]
fn ok_fails_when_its_value_does() {
    let store = |x: u32| if x < 10 { Ok(()) } else { Err(format!("rejected {x}")) };
    let reports = [
        ("false", run(|x| Ok::<_, String>(x < 10))),
        ("TestResult", run(|x| Ok::<_, String>(TestResult::check(x < 10, "too large")))),
        ("Err", run(|x| Ok::<_, String>(store(x)))),
        ("a joined thread", run(|x| thread::spawn(move || store(x)).join())),
    ];
    for (what, report) in reports {
        assert_eq!(report.status, Status::Failed, "{what}: {report}");
        assert_eq!(report.counterexample().unwrap().debug, "10", "{what}: {report}");
    }
}

#[test]
fn ok_passes_or_discards_when_its_value_does() {
    run(|_| Ok::<_, String>(())).assert_passed();
    run(|x| Ok::<_, String>(x <= 1000)).assert_passed();
    let report = run(|x| {
        Ok::<_, String>(if x % 2 == 0 { TestResult::Pass } else { TestResult::discard("odd") })
    });
    report.assert_passed();
    assert_eq!(report.discard_reasons.keys().collect::<Vec<_>>(), ["odd"], "{report}");
}

#[cfg(feature = "derive")]
#[should_panic(expected = "property returned false")]
#[ephyra::test(config = Config::default().no_regressions())]
fn the_test_attribute_fails_on_ok_false(x: u32) -> Result<bool, std::num::ParseIntError> {
    let parsed: u32 = x.to_string().parse()?;
    Ok(parsed != x)
}
