// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Replays target/crash.bin (or the file given) under the sanitizers.
fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "target/crash.bin".to_owned());
    ada_asan::replay(&std::fs::read(path).unwrap());
    eprintln!("no failure");
}
