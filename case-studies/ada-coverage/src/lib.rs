// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The ada code that ephyra-ada's cases exercise, read in-process from an
//! instrumented ada (see `.cargo/config.toml`).

use std::collections::BTreeSet;

use ephyra::coverage::{self, Functions};

/// Whether `name`, a symbol from the coverage map, is a function of ada's
/// own C++ code: Itanium-mangled in the `ada` namespace, const members
/// included. File-local functions carry a `file:` prefix, dropped first.
/// This leaves out ephyra-ada's shim and the standard library
/// instantiations.
pub fn is_ada(name: &str) -> bool {
    let name = name.rsplit(':').next().unwrap_or(name);
    name.starts_with("_ZN3ada") || name.starts_with("_ZNK3ada")
}

/// The counters of every ada function.
pub fn ada_counters() -> BTreeSet<usize> {
    Functions::get()
        .all()
        .iter()
        .filter(|f| is_ada(&f.name))
        .flat_map(|f| f.counters.clone())
        .collect()
}

/// The `within` counters that running `f` executes.
pub fn covered(within: &BTreeSet<usize>, f: impl FnOnce()) -> BTreeSet<usize> {
    let before = coverage::snapshot().expect("an instrumented ada");
    f();
    let delta = before.delta(&coverage::snapshot().expect("an instrumented ada"));
    delta.hits().iter().map(|(c, _)| *c).filter(|c| within.contains(c)).collect()
}
