// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The coverage report of a library, from a separate test crate.

#![cfg(all(ephyra_instrumented, feature = "coverage"))]

use ephyra::{
    Config,
    GenExt,
    check,
    config::CoverageTarget,
    coverage::{
        Functions,
        Summary,
    },
    gens,
};

/// The coverage of a property that calls `label` of `ephyra::runner::labels`
/// and nothing else of that module.
fn coverage(filter: &str, targets: &[&str]) -> Summary {
    let mut config = Config::default().tests(10).seed(1).no_regressions();
    config.coverage.filter = filter.to_string();
    config.coverage.targets =
        targets.iter().map(|file| CoverageTarget::new(*file, 1..=u32::MAX)).collect();
    config.coverage.targets_file = None;
    let report = check(&config, gens::int(0i32..=9).filter("not 3", |x| *x != 3), |x| {
        ephyra::label(if x > 4 { "large" } else { "small" });
        true
    });
    report.coverage.expect("instrumented")
}

fn lists(functions: &[String], function: &str) -> bool {
    functions.iter().any(|name| {
        name.strip_prefix(function).is_some_and(|rest| rest.is_empty() || rest.starts_with("::<"))
    })
}

#[test]
fn functions_that_nothing_calls_are_never_reached() {
    let coverage = coverage("ephyra::runner::labels", &[]);
    assert!(lists(&coverage.unreached, "ephyra::runner::labels::target"), "{coverage:?}");
    let label = lists(&coverage.unreached, "ephyra::runner::labels::label");
    assert!(!label, "`label::<&str>` ran, so `label::<_>` did: {coverage:?}");
    assert!(coverage.functions_total >= coverage.functions_reached + coverage.unreached.len());
}

#[test]
fn targets_add_functions_that_nothing_calls() {
    let files = ["src/runner/labels.rs", "src/gens/mod.rs", "src/lib.rs"];
    let coverage = coverage("ephyra::runner::labels::label", &files);
    let outside_the_filter = "ephyra::runner::labels::target";
    assert!(lists(&coverage.unreached, outside_the_filter), "{coverage:?}");
    let [_, gens, lib] = &coverage.targets[..] else { panic!("{coverage:?}") };
    assert!(gens.functions.iter().any(|name| name.contains("GenExt>::filter::<")), "{gens:?}");
    let uninstantiated = lists(&gens.functions, "<_ as ephyra::gens::GenExt>::filter");
    assert!(!uninstantiated, "another instantiation has code: {gens:?}");
    // Every dependency has a `src/lib.rs`.
    let with_code = |name: &String| Functions::get().all().iter().any(|f| f.name == *name);
    for name in lib.functions.iter().filter(|name| !name.contains("ephyra")) {
        assert!(with_code(name), "nothing calls {name} of a dependency");
    }
}
