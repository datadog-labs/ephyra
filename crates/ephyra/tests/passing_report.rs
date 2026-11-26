// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! What the terminal shows of a passing property. Each test runs one of the
//! ignored properties below in a child process, under the test harness.
#![cfg(feature = "derive")]

use std::process::Command;

use ephyra::{
    Config,
    cover,
};

#[ephyra::test(tests = 50, config = Config::default().no_regressions())]
#[ignore = "run in a child process"]
fn quiet(x: u8) {
    let _ = x;
}

#[ephyra::test(tests = 50, config = Config::default().no_regressions())]
#[ignore = "run in a child process"]
fn uncovered(x: u8) {
    cover(u16::from(x) > 255, "two bytes");
}

#[cfg(feature = "json")]
#[ephyra::test(tests = 50, gen = ephyra::gens::constant(2))]
#[ignore = "run in a child process"]
fn remembered(x: u8) {
    let _ = x;
}

/// Standard output and standard error of the test `name`, which must pass.
fn run(name: &str, flags: &[&str], env: &[(&str, &str)]) -> (String, String) {
    let mut command = Command::new(std::env::current_exe().unwrap());
    for (key, _) in std::env::vars() {
        if key.starts_with("EPHYRA_") || key.starts_with("RUST_TEST_") {
            command.env_remove(key);
        }
    }
    let output = command
        .args([name, "--exact", "--ignored"])
        .args(flags)
        // An instrumented build has warnings of its own.
        .env("EPHYRA_COVERAGE", "0")
        .envs(env.iter().copied())
        .output()
        .unwrap();
    let [stdout, stderr] =
        [output.stdout, output.stderr].map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
    assert!(output.status.success() && stdout.contains("1 passed"), "{stdout}\n{stderr}");
    (stdout, stderr)
}

#[test]
fn a_passing_report_is_shown_on_request() {
    let (stdout, stderr) = run("quiet", &[], &[]);
    assert!(!stdout.contains("ephyra:") && !stderr.contains("ephyra:"), "{stdout}\n{stderr}");
    for flag in ["--nocapture", "--show-output"] {
        let (stdout, _) = run("quiet", &[flag], &[]);
        assert!(stdout.contains("ephyra: ok, 50 tests passed"), "{stdout}");
    }
}

#[test]
fn warnings_about_what_was_asked_are_not_captured() {
    let env = [("EPHYRA_TESTS", "1_000_000"), ("EPHYRA_SEEDS", "3"), ("EPHYRA_REPLAY", "1:2")];
    let (stdout, stderr) = run("uncovered", &[], &env);
    assert!(!stdout.contains("ephyra:"), "{stdout}");
    for line in [
        "\nephyra: `uncovered` passed, with warnings",
        "  replay point 1:2 did not fail",
        "  warning [config_env]: EPHYRA_SEEDS is not a ephyra setting",
        "  warning [config_env]: EPHYRA_TESTS=1_000_000: expected an integer, ignored",
        "  warning [cover]: cover `two bytes` was met by 0.0% of passing tests",
    ] {
        assert!(stderr.contains(line), "missing `{line}` in:\n{stderr}");
    }
}

#[cfg(feature = "json")]
#[test]
fn a_stale_regression_is_not_captured() {
    let dir = tempfile::tempdir().unwrap();
    let config = Config::default().name("remembered").regressions_dir(dir.path());
    let report = ephyra::check(&config, ephyra::gens::constant(1u8), |_| false);
    assert!(report.regressions.saved, "{report}");

    let dir = dir.path().to_str().unwrap();
    let (_, stderr) = run("remembered", &[], &[("EPHYRA_REGRESSIONS_DIR", dir)]);
    assert!(stderr.contains("ephyra: `remembered` passed, with warnings"), "{stderr}");
    assert!(stderr.contains("no longer matches the generator"), "{stderr}");
}
