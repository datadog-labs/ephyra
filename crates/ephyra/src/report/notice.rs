// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! What a passing run says, also where its report is not shown.

use std::{
    collections::BTreeSet,
    io::Write,
    sync::{
        Mutex,
        PoisonError,
    },
};

use super::Report;

/// Health warnings that stay in the report of a passing run, so that suites
/// do not get noisier: advice about the property or the build, and
/// `coverage_target_missed`, because the tests of a suite share one targets
/// file and each misses the targets meant for another.
///
/// Every other kind means that the run did not do what it was asked: a
/// setting was ignored, a stored input was not replayed, a `cover` was not
/// met, a filter selects nothing, a file was not read or written, a sink
/// failed. A kind added later is shown unless it is added here.
const ADVICE: [&str; 8] = [
    "coverage_plateau",
    "coverage_target_missed",
    "coverage_threads",
    "discard_ratio",
    "locate_unavailable",
    "low_distinct_inputs",
    "mutator_inert",
    "slow_generation",
];

/// Whether the properties of a process share the cause of warnings of `kind`:
/// a variable, a file, a sink or the build.
fn shared(kind: &str) -> bool {
    matches!(kind, "config_env" | "observations" | "profraw" | "report_json")
        || kind.starts_with("coverage")
}

/// The messages with a [`shared`] cause that were shown. Every property of a
/// suite can have the same one, so each is shown once.
static SHOWN: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

/// Print the report of a passing run, which the test harness shows on
/// request. `#[ephyra::test]` calls it, because the crate of the test may
/// have no `std`.
pub fn __print_passed(report: &Report) {
    if report.passed() {
        println!("{report}");
    }
}

/// Write what a passing `report` must say to standard error.
pub(super) fn show(report: &Report) {
    let text = text(report, &mut SHOWN.lock().unwrap_or_else(PoisonError::into_inner));
    // Not `eprint!`: the test harness captures it and drops it when the test
    // passes. One write, so that parallel tests do not mix their lines.
    let _ = std::io::stderr().write_all(text.as_bytes());
}

/// The block for `report`, empty when it has nothing to say. It starts with
/// a line break because the harness may have left a line open.
fn text(report: &Report, shown: &mut BTreeSet<String>) -> String {
    let mut lines = String::new();
    if let Some(replay) = report.replay.as_ref().filter(|replay| !replay.reproduced) {
        lines.push_str(&format!("  replay point {} did not fail\n", replay.point));
    }
    for warning in &report.health {
        let kind = warning.kind.as_str();
        if ADVICE.contains(&kind) || (shared(kind) && !shown.insert(warning.message.clone())) {
            continue;
        }
        lines.push_str(&format!("  warning [{kind}]: {}\n", warning.message));
    }
    if lines.is_empty() {
        return lines;
    }
    format!("\nephyra: `{}` passed, with warnings\n{lines}", report.name)
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::{
            ReplayPoint,
            report::ReplayOutcome,
        },
    };

    fn report(name: &str, warnings: &[(&str, &str)]) -> Report {
        let mut report = Report { name: name.to_string(), ..Report::new(1, 100) };
        for (kind, message) in warnings {
            report.warn(kind, *message);
        }
        report
    }

    #[test]
    fn advice_stays_in_the_report() {
        let shown = &mut BTreeSet::new();
        assert_eq!(text(&report("p", &[]), shown), "");
        assert_eq!(text(&report("p", &ADVICE.map(|kind| (kind, "advice"))), shown), "");
        let mixed = [("slow_generation", "slow"), ("cover", "unmet"), ("regressions", "stale")];
        assert_eq!(
            text(&report("a::p", &mixed), shown),
            "\nephyra: `a::p` passed, with warnings\n  warning [cover]: unmet\n  warning [regressions]: stale\n"
        );
    }

    #[test]
    fn problems_that_properties_share_are_shown_once() {
        let shown = &mut BTreeSet::new();
        let warnings = [
            ("config_env", "EPHYRA_TESTS=1_0"),
            ("coverage_filter", "few"),
            ("cover", "unmet"),
            ("report_json", "read only"),
            ("observations", "no agent"),
        ];
        let first = text(&report("first", &warnings), shown);
        assert_eq!(first.lines().count(), 7, "{first}");
        let second = text(&report("second", &warnings), shown);
        assert_eq!(second, "\nephyra: `second` passed, with warnings\n  warning [cover]: unmet\n");
        let other = text(&report("third", &[("config_env", "EPHYRA_SEED=x")]), shown);
        assert!(other.contains("EPHYRA_SEED=x"), "{other}");
    }

    #[test]
    fn a_replay_point_that_did_not_fail_is_shown() {
        let point = ReplayPoint { seed: 7, iteration: 3 };
        let replayed = |reproduced| {
            Report { replay: Some(ReplayOutcome { point, reproduced }), ..report("p", &[]) }
        };
        let passed = text(&replayed(false), &mut BTreeSet::new());
        assert!(passed.contains("replay point 7:3 did not fail"), "{passed}");
        assert_eq!(text(&replayed(true), &mut BTreeSet::new()), "");
    }
}
