// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The sanitized build catches errors in C++ and saves the case: a child
//! process reads past a heap buffer inside the shim, while a known case
//! stands as the current one.

use std::process::Command;

use ephyra_ada::{Case, Setter};

extern "C" {
    fn ea_canary(bytes: *const u8, len: usize) -> i32;
}

#[test]
fn heap_overflow_is_caught_and_the_case_saved() {
    if std::env::var_os("EA_CANARY_CHILD").is_some() {
        // The shim's encoding of the case below, as ea_run would see it.
        let bytes =
            b"\x02\x00\x00\x00C|\x01\x05\x00\x00\x00file:\x01\x00\x00\x00\x05\x01\x00\x00\x00h";
        // SAFETY: the canary misbehaves on purpose; the sanitizer ends the
        // process before this returns.
        unsafe { ea_canary(bytes.as_ptr(), bytes.len()) };
        return;
    }
    let crash = std::env::temp_dir().join(format!("ada-asan-canary-{}.bin", std::process::id()));
    let out = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "heap_overflow_is_caught_and_the_case_saved", "--nocapture"])
        .env("EA_CANARY_CHILD", "1")
        .env("EA_CRASH_FILE", &crash)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "the child survived:\n{stderr}");
    // UBSan's object-size check sees the read before ASan does; either
    // report will do.
    assert!(
        stderr.contains("heap-buffer-overflow") || stderr.contains("runtime error"),
        "no sanitizer report:\n{stderr}"
    );
    let saved = std::fs::read(&crash).expect("the crash file");
    let (_, _, case) = ada_asan::read_crash(&saved);
    assert_eq!(
        case,
        Case {
            input: "C|".into(),
            base: Some("file:".into()),
            sets: vec![(Setter::Hostname, "h".into())]
        }
    );
    std::fs::remove_file(crash).unwrap();
}
