// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

fn main() {
    let dir = std::env::var("DUCKDB_LIB_DIR").expect("set DUCKDB_LIB_DIR to <build>/cmake/src");
    println!("cargo:rustc-link-search=native={dir}");
    println!("cargo:rustc-link-lib=dylib=duckdb");
    println!("cargo:rerun-if-env-changed=DUCKDB_LIB_DIR");
}
