// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The generator contract, shared by `gen_contract.rs` (the built-in generators) and
//! `gen_contract_derived.rs` (derived recursive types). See `gen_contract.rs` for the
//! clauses.
#![allow(dead_code)]

use std::{
    collections::HashSet,
    fmt::Debug,
    ops::ControlFlow,
};

use ephyra::{
    Gen,
    Source,
};

/// Mutation steps followed from each start.
pub const CHAIN: usize = 48;
/// Shrink candidates checked per value.
pub const CANDIDATES: usize = 48;
/// Steps of the shrink walk: a correct shrinker settles, or the walk ends here.
pub const WALK: usize = 400;
/// Draws checked for spread (6): from a support of two values, both, all but surely.
pub const DRAWS: usize = 64;

/// Checks 1-6 for `generator` from one seed and size. `starts` are values of the support
/// the chains and walks also start from: its edges, where bugs live.
pub fn keeps_contract<G>(
    generator: &G,
    support: impl Fn(&G::Value) -> bool,
    starts: &[G::Value],
    (seed, size): (u64, usize),
) -> Result<(), String>
where
    G: Gen,
    G::Value: Clone + Debug,
{
    let show = |v: &G::Value| format!("{v:?}");
    // 1, 2
    let mut source = Source::new(seed, size);
    let first = generator.generate(&mut source);
    if source.discarded().is_some() {
        return Ok(());
    }
    let again = generator.generate(&mut Source::new(seed, size));
    if show(&first) != show(&again) {
        return Err(format!("seed {seed} generated {first:?}, then {again:?}"));
    }
    if !support(&first) {
        return Err(format!("generated {first:?}, outside the support"));
    }
    // 6
    if starts.iter().map(show).collect::<HashSet<_>>().len() > 1 {
        let mut draws = Source::new(seed ^ 0x7370_7265_6164, size);
        let mut seen = HashSet::new();
        for _ in 0..DRAWS {
            let v = generator.generate(&mut draws);
            if draws.discarded().is_some() {
                break;
            }
            seen.insert(show(&v));
        }
        if seen.len() == 1 {
            return Err(format!(
                "{DRAWS} draws all gave {first:?}, though the support holds {starts:?}"
            ));
        }
    }
    let mut g = Source::new(seed ^ 0x636f_6e74_7261_6374, size);
    for start in std::iter::once(&first).chain(starts) {
        // 3, 4: along a mutation chain, every value and its shrink candidates
        let mut value = start.clone();
        for step in 0..CHAIN {
            let mut problem = None;
            let mut seen = 0;
            let _ = generator.shrink(&value, &mut |c| {
                seen += 1;
                if show(&c) == show(&value) {
                    problem = Some(format!("{value:?} shrank to itself"));
                } else if !support(&c) {
                    problem = Some(format!("{value:?} shrank to {c:?}, outside the support"));
                }
                if problem.is_some() || seen >= CANDIDATES {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            });
            if let Some(p) = problem {
                return Err(p);
            }
            let next = generator.mutate(&value, &mut g);
            if g.discarded().is_some() {
                g = Source::new(g.int(0u64..=u64::MAX), size);
                break;
            }
            if !support(&next) {
                return Err(format!(
                    "mutating {value:?} (step {step} from {start:?}) gave {next:?}, outside the support"
                ));
            }
            value = next;
        }
        // 5: a random walk over shrink candidates; a revisit is a cycle
        let mut value = start.clone();
        let mut visited = HashSet::from([show(&value)]);
        for _ in 0..WALK {
            let candidates = generator.shrink_candidates(&value);
            if candidates.is_empty() {
                break;
            }
            let next = candidates[g.int(0..candidates.len())].clone();
            if !visited.insert(show(&next)) {
                return Err(format!(
                    "shrinking from {start:?} cycles: {value:?} -> {next:?} again"
                ));
            }
            value = next;
        }
    }
    Ok(())
}
