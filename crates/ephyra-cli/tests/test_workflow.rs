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

/// `ephyra test` in `dir`, with no rustc flags from the machine.
fn without_flags(dir: &Path) -> Command {
    let mut command = ephyra_test(dir);
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().ends_with("RUSTFLAGS") || name == "CARGO_BUILD_TARGET" {
            command.env_remove(name);
        }
    }
    command.env("CARGO_HOME", dir.join("home"));
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

/// Instrumented builds get the rustc flags of a plain `cargo test`, wherever
/// cargo takes them from, and `-C instrument-coverage`. To find them out does
/// not change the project.
#[test]
fn builds_add_instrumentation_to_the_flags_cargo_uses() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for (file, text) in [
        (
            "Cargo.toml",
            "[package]\nname=\"flags_consumer\"\nversion=\"0.0.0\"\nedition=\"2024\"\n[workspace]\n",
        ),
        ("src/lib.rs", ""),
        (
            "build.rs",
            "fn main() {\nprintln!(\"cargo::rerun-if-env-changed=CARGO_ENCODED_RUSTFLAGS\");\n\
             let flags = std::env::var(\"CARGO_ENCODED_RUSTFLAGS\").unwrap();\n\
             std::fs::write(\"flags\", flags.replace('\\x1f', \"|\")).unwrap();\n}\n",
        ),
    ] {
        let path = root.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    let config = "[build]\nrustflags=[\"--cfg\", \"words=\\\"one two\\\"\"]\n";
    let target = ["--config", "target.'cfg(all())'.rustflags=[\"--cfg\", \"of_the_target\"]"];
    let built = root.join("built");
    let exported = ("RUSTFLAGS", "--cfg a");
    let encoded = ("CARGO_ENCODED_RUSTFLAGS", "--cfg\x1fwords=\"b c\"");
    fs::create_dir(root.join(".cargo")).unwrap();
    for (config, arguments, variables, flags) in [
        ("", &[][..], &[][..], ""),
        // Each of these replaces what comes before it.
        (config, &["--locked"], &[], "--cfg|words=\"one two\"|"),
        (config, &target, &[], "--cfg|of_the_target|"),
        (config, &target, &[exported], "--cfg|a|"),
        (config, &target, &[exported, encoded], "--cfg|words=\"b c\"|"),
    ] {
        fs::write(root.join(".cargo/config.toml"), config).unwrap();
        let (code, output) = run(without_flags(root)
            .arg("--target-dir")
            .arg(&built)
            .args(arguments)
            .arg("--no-run")
            .env("CARGO_RESOLVER_LOCKFILE_PATH", root.join("locked/Cargo.lock"))
            .envs(variables.iter().copied()));
        assert_eq!(code, Some(0), "{output}");
        assert_eq!(
            fs::read_to_string(root.join("flags")).unwrap(),
            format!("{flags}-C|instrument-coverage"),
            "{output}"
        );
    }
    assert!(built.join("CACHEDIR.TAG").exists(), "cargo did not create the target directory");
    // Cargo runs what a build directory holds, and `.ephyra` can come from elsewhere.
    assert!(!root.join(".ephyra/rustflags/target").exists(), "cargo builds in the artifacts");
}

#[test]
fn a_run_goes_on_when_cargo_does_not_tell_its_flags() {
    let dir = tempfile::tempdir().unwrap();
    let (code, output) =
        run(without_flags(dir.path()).args(["--cargo", "/bin/echo", "--test", "props"]));
    assert_eq!(code, Some(0), "{output}");
    assert!(output.contains("could not read rustflags from cargo's configuration"), "{output}");
    assert!(output.contains("\ntest --test props\n"), "{output}");
}
