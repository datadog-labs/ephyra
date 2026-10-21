// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The runner's build-time version (`RUNNER_BUILT_VERSION`): the content
//! hash of the sources this build compiles, so that a run records what its
//! binary was built from (see `runner version`). Only the runner reads it;
//! the library, `qscore` and `functions` are unaffected.

#[path = "src/bin/runner/content.rs"]
mod content;

use std::path::Path;

fn main() {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    for (_, dir) in content::components(here) {
        for f in content::sources(&dir) {
            println!("cargo:rerun-if-changed={}", f.display());
        }
    }
    println!("cargo:rustc-env=RUNNER_BUILT_VERSION={}", content::version_id(here));
}
