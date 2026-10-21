// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Compiles ada from `vendor/` (the amalgamation of ada main; `vendor/COMMIT`
//! says which commit), URLPattern included, and `cpp/shim.cpp`, which runs
//! cases on both of ada's URL representations.
//!
//! `EA_VENDOR=<dir>` compiles the amalgamation in `<dir>` instead; the bug
//! benchmark (`bench/`) uses it to build ada with a known bug put back. Give
//! each such build its own `CARGO_TARGET_DIR`.
//!
//! ada's development checks (`ADA_DEVELOPMENT_CHECKS`) are off: their
//! assertions abort the process, losing the case. The shim calls
//! `url_aggregator::validate()`, the check they mostly make, after every step
//! instead. Coverage builds add their flags through `CXXFLAGS`, which `cc`
//! honours.

fn main() {
    println!("cargo:rerun-if-env-changed=EA_VENDOR");
    let vendor = std::env::var("EA_VENDOR").unwrap_or_else(|_| "vendor".to_owned());
    for f in [format!("{vendor}/ada.cpp"), format!("{vendor}/ada.h"), "cpp/shim.cpp".to_owned()] {
        println!("cargo:rerun-if-changed={f}");
    }
    cc::Build::new()
        .cpp(true)
        .std("c++20")
        .include(&vendor)
        // URLPattern with the standard library's regex: ada calls the
        // provider unsafe (std::regex can be slow or recurse deeply on
        // hostile patterns and inputs), which the generated cases avoid.
        .define("ADA_INCLUDE_URL_PATTERN", "1")
        .define("ADA_USE_UNSAFE_STD_REGEX_PROVIDER", None)
        .define("ADA_DEVELOPMENT_CHECKS", "0")
        .flag_if_supported("-Wno-unused-function")
        .file(format!("{vendor}/ada.cpp"))
        .file("cpp/shim.cpp")
        .compile("ada");
}
