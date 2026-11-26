// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! `ephyra test` end to end, on a throwaway consumer package: it runs the
//! project as `cargo test` does.
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

/// `ephyra test` in `dir`, with options and cargo arguments to add.
fn ephyra_test(dir: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ephyra"));
    command
        .arg("test")
        // Shared with the check workflow, so the ephyra library builds once.
        .args(["--target-dir", concat!(env!("CARGO_TARGET_TMPDIR"), "/check-workflow")])
        .env_remove("EPHYRA_REGRESSIONS_DIR")
        .current_dir(dir);
    command
}

fn run(command: &mut Command) -> (Option<i32>, String) {
    let Output { status, stdout, stderr } = command.output().unwrap();
    let output =
        format!("{}\n{}", String::from_utf8_lossy(&stdout), String::from_utf8_lossy(&stderr));
    (status.code(), output)
}

#[test]
fn saved_failures_stay_where_cargo_test_keeps_them() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    common::write_consumer(root, "saved_failures_consumer");
    let source = "#[ephyra::test]\nfn never(x: u8) -> bool { x.count_ones() > 8 }\n";
    fs::write(root.join("tests/props.rs"), source).unwrap();

    // As from a workspace root: not the directory `cargo test` runs tests in.
    let here = root.join("tests");
    let (code, output) = run(ephyra_test(&here).args(["--no-instrument", "--test", "props"]));
    assert_eq!(code, Some(101), "{output}");
    assert!(here.join(".ephyra/reports.jsonl").exists(), "{output}");
    assert!(root.join(".ephyra/regressions/never.jsonl").exists(), "{output}");

    let chosen = root.join("chosen");
    let (code, output) = run(ephyra_test(&here)
        .args(["--no-instrument", "--test", "props"])
        .env("EPHYRA_REGRESSIONS_DIR", &chosen));
    assert_eq!(code, Some(101), "{output}");
    assert!(chosen.join("never.jsonl").exists(), "{output}");
}
