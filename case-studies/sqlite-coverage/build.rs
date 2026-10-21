//! The runner's build-time version (`RUNNER_BUILT_VERSION`): the content
//! hash of the sources this build compiles, so that a run records what its
//! binary was built from, not only what is on disk when it starts (a
//! rebuild that finds nothing to do, while new sources are still being
//! copied, leaves an older binary; see `runner version`).

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
    println!("cargo:rerun-if-changed={}", content::amalgamation(here).display());
    println!("cargo:rustc-env=RUNNER_BUILT_VERSION={}", content::version_id(here));
}
