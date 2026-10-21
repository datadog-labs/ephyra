//! `ephyra check` end to end, on a throwaway consumer package: it lists
//! what the source index finds, runs generator and strategy checks, reports
//! predicates that still need a generator, and fails a broken generator.
#![cfg(unix)]

mod common;

use std::{
    fs,
    path::Path,
    process::{
        Command,
        Output,
    },
};

const PROPERTIES: &str = r#"
use ephyra::{gens, Gen, TestResult};

#[ephyra::generator]
fn positive(value: &u8) -> bool {
    *value > 0
}

#[ephyra::generator(positive, metrics = byte_metrics)]
fn positive_bytes() -> impl Gen<Value = u8> {
    gens::int(LOW..=255)
}

fn byte_metrics(value: &u8, _size: usize) -> Vec<(&'static str, f64)> {
    vec![("byte value", f64::from(*value))]
}

// A predicate no generator uses yet: what `needs a generator` reports. The
// consumer is built with the caller's RUSTFLAGS, `-D warnings` in CI.
#[allow(dead_code)]
#[ephyra::generator]
fn even(value: &u8) -> bool {
    value % 2 == 0
}

fn ordered_pairs() -> impl Gen<Value = (u8, u8)> {
    gens::dependent(gens::int(0u8..=254), |lo| gens::int(*lo + 1..=255))
}

#[ephyra::test(tests = 20, gen = ordered_pairs())]
fn midpoints_lie_between(lo: u8, hi: u8) -> TestResult {
    ephyra::assume!(lo < hi);
    let mid = lo + (hi - lo) / 2;
    TestResult::check(lo <= mid && mid < hi, "the midpoint left the range")
}

// Never discards, so `positive_bytes`' own check covers its strategy.
#[ephyra::test(tests = 20)]
fn doubling_keeps_positive(#[ephyra(gen = positive_bytes())] byte: u8) {
    assert!(u16::from(byte) * 2 > 0);
}
"#;

fn write_properties(root: &Path, low: u8) {
    fs::write(root.join("tests/props.rs"), PROPERTIES.replace("LOW", &format!("{low}u8"))).unwrap();
}

fn ephyra(root: &Path, args: &[&str]) -> (Option<i32>, String) {
    let Output { status, stdout, stderr } = Command::new(env!("CARGO_BIN_EXE_ephyra"))
        .args(args)
        // Shared across runs, so the ephyra library builds once.
        .args(["--target-dir", concat!(env!("CARGO_TARGET_TMPDIR"), "/check-workflow")])
        .current_dir(root)
        .output()
        .unwrap();
    let output =
        format!("{}\n{}", String::from_utf8_lossy(&stdout), String::from_utf8_lossy(&stderr));
    (status.code(), output)
}

#[test]
fn check_lists_runs_and_fails_generators_and_strategies() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    common::write_consumer(root, "check_workflow_consumer");
    write_properties(root, 1);

    let (code, listed) = ephyra(root, &["check", "--list"]);
    assert_eq!(code, Some(2), "{listed}");
    for line in [
        "predicate  positive (",
        "generator  positive_bytes (",
        "generator of u8 for `positive`",
        "property   midpoints_lie_between (",
        "with strategy `ordered_pairs()`",
        "property   doubling_keeps_positive (",
        "with strategy `positive_bytes()` (never discards, so not checked)",
        "needs a generator: even (",
    ] {
        assert!(listed.contains(line), "missing `{line}` in:\n{listed}");
    }
    assert!(!listed.contains("== checking"), "--list builds nothing:\n{listed}");

    let (code, checked) = ephyra(root, &["check", "--no-instrument", "--tests", "300"]);
    assert_eq!(code, Some(2), "checks pass, but `even` needs a generator:\n{checked}");
    assert!(checked.contains("ephyra check: 2 passed, 0 failed"), "{checked}");
    assert!(checked.contains("byte value"), "the metrics are reported:\n{checked}");
    assert!(checked.contains("300 tests passed"), "{checked}");
    let reports = fs::read_to_string(root.join(".ephyra/reports.jsonl")).unwrap();
    assert!(reports.contains("\"positive_bytes\""), "{reports}");

    write_properties(root, 0);
    let (code, failed) =
        ephyra(root, &["check", "positive_bytes", "--no-instrument", "--tests", "300"]);
    assert_eq!(code, Some(1), "{failed}");
    // Zero shows up as a generated value or as a shrink candidate of one.
    assert!(failed.contains("the predicate returned false"), "{failed}");
    assert!(failed.contains("counterexample:"), "{failed}");
    assert!(failed.contains("ephyra check: 0 passed, 1 failed"), "{failed}");

    let (code, unknown) = ephyra(root, &["check", "no_such_generator", "--list"]);
    assert_eq!(code, Some(1), "{unknown}");
}
