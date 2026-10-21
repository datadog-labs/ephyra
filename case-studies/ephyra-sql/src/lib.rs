//! A SQL testing engine for Turso, built as an ephyra loop: SQL cases from a
//! typed AST whose generators, shrinkers and mutators ephyra checks, run
//! against Turso and SQLite under differential and metamorphic oracles. See
//! `README.md` for the design.

pub mod ast;
pub mod engine;
pub mod generate;
pub mod observe;
pub mod script;

use std::fmt::Debug;

use ephyra::{Config, Gen, Run, Testable, Trace};

/// The configuration every property runs under: coverage tracks
/// `turso_core`, not this crate, so the coverage page, the novelty the loop
/// climbs and fault localization are all about the engine.
pub fn config() -> Config {
    let mut config = Config::default();
    config.coverage.filter = "turso_core".into();
    config
}

/// The first loop: alternate fresh cases with mutants of cases that reached
/// new coverage, each seed weighted by how much it reached. Without an
/// instrumented build nothing is novel, so this is random testing.
pub fn explore<G, R, P>(run: &mut Run<G, P>)
where
    G: Gen,
    G::Value: Clone + Debug,
    R: Testable,
    P: FnMut(G::Value) -> R,
{
    let mut mutate = false;
    while run.wants_more() {
        mutate = !mutate;
        let parent = if mutate { run.pool_pick() } else { None };
        let input = match &parent {
            Some(parent) => run.mutate(parent),
            None => run.draw(),
        };
        let Some(input) = input else { continue };
        let tested = run.test(&input);
        if tested.failed() {
            break;
        }
        if tested.novelty() > 0 {
            run.keep_always(input.value, f64::from(tested.novelty()));
        }
    }
}

/// [`explore`], then, if a trace failed, causality analysis of it: the
/// failing script is rewound to earlier steps and continued from each many
/// times, and the report shows after which step the failure became likely.
pub fn explore_and_explain<G, R, P>(run: &mut Run<G, P>)
where
    G: Trace,
    G::Value: Clone + Debug,
    R: Testable,
    P: FnMut(G::Value) -> R,
{
    explore(run);
    if run.failure().is_some() {
        ephyra::loops::causality(run, 32, 12);
    }
}
