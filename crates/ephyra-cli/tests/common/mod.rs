// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Helpers shared by the end-to-end `ephyra` workflow tests.

use std::{
    fs,
    path::Path,
};

/// A throwaway package at `root`, with an empty `tests/` directory, that
/// depends on the ephyra library next to this crate.
pub fn write_consumer(root: &Path, name: &str) {
    fs::create_dir(root.join("tests")).unwrap();
    write_manifest(root, name, "[workspace]\n");
}

/// The `Cargo.toml` of a package `name` at `root` that depends on the ephyra
/// library next to this crate, and ends in `rest`.
pub fn write_manifest(root: &Path, name: &str, rest: &str) {
    fs::write(
        root.join("Cargo.toml"),
        format!(
            "[package]\nname=\"{name}\"\nversion=\"0.0.0\"\nedition=\"2024\"\n[dependencies]\nephyra={{path={:?}}}\n{rest}",
            concat!(env!("CARGO_MANIFEST_DIR"), "/../ephyra")
        ),
    )
    .unwrap();
}
