// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Property-based testing for Rust, built for tests that are written and
//! verified by coding agents: you author the generator, the support
//! predicate, the shrinker and the mutator, and the library verifies all
//! of them and makes every failure reproducible from its seed.
//!
//! # Overview
//!
//! - [`Source`] is a seeded random source; [`Gen`] is anything that
//!   produces a value from it, together with its shrinker and mutator.
//!   [`gens`] holds the constructors and combinators, all uniform unless a
//!   bias is composed explicitly. [`Arbitrary`] is a type's default
//!   generator (`#[derive(Arbitrary)]` with the `derive` feature).
//! - [`check`] runs a property, shrinks the first failure with the
//!   generator's shrinker, remembers it in a regressions file and returns a
//!   [`Report`] with a replay point. [`quickcheck`] is `check` with defaults.
//! - [`check_generator`], [`check_shrinker`] and [`check_mutator`] verify a
//!   generator and its hooks against a support predicate.
//! - [`Config`] holds every knob; `EPHYRA_*` variables override it (see
//!   [`config`]).
//! - Inside a property, the [side channel](runner::labels) records how inputs
//!   are distributed and steers the search. The report also shows a few
//!   sampled inputs and per-input timing, and `EPHYRA_OBSERVATIONS=dir`
//!   writes every input as an OpenPBTStats observation for Tyche; a
//!   [`sink`] installed in the process receives the same inputs.
//! - With the `coverage` feature and `RUSTFLAGS="-C instrument-coverage"`,
//!   the report says which functions of the crate under test each property
//!   reached, inputs that reach new code seed the targeted search, and a
//!   failure comes with the functions ranked by how much their coverage
//!   separates failing runs from passing ones (the `coverage` module,
//!   behind the feature of the same name).
//!
//! The API is not yet frozen.
//!
//! # Checking a property
//!
//! ```
//! use ephyra::{check, gens, Config};
//!
//! let report = check(&Config::default().tests(500), gens::vec(gens::int(-50i32..=50)), |mut v| {
//!     let before = v.len();
//!     v.sort();
//!     v.len() == before && v.windows(2).all(|w| w[0] <= w[1])
//! });
//! report.assert_passed();
//! ```
//!
//! A property may return `bool`, `()`, `Result<_, E: Debug>` or
//! [`TestResult`]; a panic counts as a failure and its message and location
//! go in the report.
//!
//! # Writing a generator
//!
//! Any `Fn(&mut Source) -> T` is a generator. Draw decisions from the
//! [`Source`] and compose sub-generators with [`Source::draw`]:
//!
//! ```
//! use std::ops::ControlFlow;
//! use ephyra::{Gen, GenExt, Source};
//!
//! #[derive(Debug, Clone, PartialEq)]
//! enum Tree { Leaf, Node(Box<Tree>, i32, Box<Tree>) }
//!
//! fn bst(g: &mut Source, lo: i32, hi: i32, depth: usize) -> Tree {
//!     if depth == 0 || lo > hi || !g.bool_with(0.8) {
//!         return Tree::Leaf;
//!     }
//!     let key = g.int(lo..=hi);
//!     let left = bst(g, lo, key - 1, depth - 1);
//!     let right = bst(g, key + 1, hi, depth - 1);
//!     Tree::Node(Box::new(left), key, Box::new(right))
//! }
//!
//! // A shrinker that keeps the tree a BST: drop a subtree, or replace the
//! // node by one of its children.
//! fn shrink_bst(t: &Tree, visitor: &mut dyn FnMut(Tree) -> ControlFlow<()>) -> ControlFlow<()> {
//!     match t {
//!         Tree::Leaf => ControlFlow::Continue(()),
//!         Tree::Node(l, k, r) => {
//!             visitor((**l).clone())?;
//!             visitor((**r).clone())?;
//!             visitor(Tree::Leaf)?;
//!             shrink_bst(l, &mut |left| visitor(Tree::Node(Box::new(left), *k, r.clone())))?;
//!             shrink_bst(r, &mut |right| visitor(Tree::Node(l.clone(), *k, Box::new(right))))
//!         }
//!     }
//! }
//!
//! let trees = (|g: &mut Source| bst(g, -100, 100, 5)).with_shrink(shrink_bst);
//! let mut source = Source::new(42, 10);
//! let tree = trees.generate(&mut source);
//! // The same seed regenerates the same value.
//! assert_eq!(trees.generate(&mut Source::new(42, 10)), tree);
//! ```

#![deny(unsafe_code)]

pub mod arbitrary;
pub mod config;
#[cfg(feature = "coverage")]
pub mod coverage;
pub mod gens;
mod health;
mod hooks;
pub mod loops;
mod pool;
mod primitives;
#[cfg(feature = "json")]
pub mod regressions;
pub mod report;
pub mod runner;
mod shrink;
pub mod sink;
pub mod source;
mod verify;

pub use {
    arbitrary::{
        Arbitrary,
        any,
    },
    config::{
        Config,
        ReplayPoint,
    },
    gens::{
        BoxGen,
        Gen,
        GenExt,
        Trace,
    },
    hooks::{
        ShrinkFlow,
        ShrinkVisitor,
    },
    report::{
        CausalJump,
        CausalPoint,
        Causality,
        Report,
        ShrinkStats,
        Status,
        TargetSummary,
    },
    runner::{
        Input,
        Outcome,
        Run,
        TestResult,
        Testable,
        Tested,
        check,
        labels::{
            FeatureValue,
            classify,
            cover,
            feature,
            label,
            measure,
            observe,
            target,
        },
        quickcheck,
    },
    source::Source,
    verify::{
        check_generator,
        check_mutator,
        check_shrinker,
    },
};

#[doc(hidden)]
pub use runner::{
    __check_generator,
    __check_strategy,
};

/// Derive a default generator for a struct or enum. Needs the `derive`
/// feature (on by default). See [`ephyra_macros::Arbitrary`].
#[cfg(feature = "derive")]
pub use ephyra_macros::Arbitrary;
/// Convert an eager `fn(&T) -> Vec<T>` shrinker into the lazy visitor API.
/// Needs the `derive` feature (on by default). See [`ephyra_macros::shrinker`].
#[cfg(feature = "derive")]
pub use ephyra_macros::shrinker;
/// Turn a function with typed parameters into a property-based `#[test]`.
/// Needs the `derive` feature (on by default). See [`ephyra_macros::test`].
#[cfg(feature = "derive")]
pub use ephyra_macros::test;

/// Mark a predicate that needs a generator, or a generator (of a predicate)
/// that `ephyra check` checks. Needs the `derive` feature (on by
/// default). See [`ephyra_macros::generator`].
#[cfg(feature = "derive")]
pub use ephyra_macros::generator;

/// A file-name-safe stem for a property name, shared by the regressions,
/// observation, profraw and coverage page files.
#[cfg(any(feature = "json", feature = "coverage"))]
pub(crate) fn file_stem(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' { c } else { '_' })
        .collect();
    while out.contains("__") {
        out = out.replace("__", "_");
    }
    let trimmed = out.trim_matches('_');
    if trimmed.is_empty() { "unnamed".to_string() } else { trimmed.to_string() }
}

/// Write `bytes` to `<dir>/<stem of name>.<ext>`, creating `dir`.
#[cfg(feature = "coverage")]
pub(crate) fn write_named(
    dir: &std::path::Path,
    name: &str,
    ext: &str,
    bytes: &[u8],
) -> std::io::Result<std::path::PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{}.{ext}", file_stem(name)));
    std::fs::write(&path, bytes)?;
    Ok(path)
}

/// Everything a test module usually needs.
pub mod prelude {
    pub use crate::{
        Arbitrary,
        BoxGen,
        Config,
        Gen,
        GenExt,
        Report,
        Source,
        TestResult,
        any,
        assume,
        check,
        check_generator,
        check_mutator,
        check_shrinker,
        classify,
        cover,
        feature,
        gens,
        label,
        measure,
        observe,
        quickcheck,
        target,
    };
}
