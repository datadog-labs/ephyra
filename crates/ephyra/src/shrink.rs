// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The shrink driver: greedy descent over a generator's candidates.
//!
//! Starting from the failing input, ask the generator for candidates, take
//! the first one that still fails with the same signature and is not
//! rendered longer than the current best, and repeat until no candidate is
//! accepted or the budget runs out. A seen-set of `Debug` renderings stops
//! cycles; the step and time budget bound the wall clock.

use std::{
    collections::HashSet,
    fmt::Debug,
    hash::{
        BuildHasher,
        RandomState,
    },
    ops::ControlFlow,
    time::Instant,
};

use crate::{
    config::ShrinkConfig,
    report::{
        Location,
        ShrinkStats,
    },
};

/// What the oracle returns for a candidate that fails the same way: the
/// failure's message and location.
pub(crate) type SameFailure = Option<(Option<String>, Option<Location>)>;

/// Result of shrinking: the best value, the failure it produced (if a
/// candidate was accepted), and the statistics.
pub(crate) struct Shrunk<T> {
    pub value: T,
    pub failure: SameFailure,
    pub stats: ShrinkStats,
}

/// The first candidate a shrinker visits, without producing the rest.
pub(crate) fn first_candidate<T>(
    shrink: impl FnOnce(&mut dyn FnMut(T) -> ControlFlow<()>) -> ControlFlow<()>,
) -> Option<T> {
    let mut first = None;
    let _ = shrink(&mut |candidate| {
        first = Some(candidate);
        ControlFlow::Break(())
    });
    first
}

/// Every candidate a shrinker visits, in order.
pub(crate) fn collect_candidates<T>(
    shrink: impl FnOnce(&mut dyn FnMut(T) -> ControlFlow<()>) -> ControlFlow<()>,
) -> Vec<T> {
    let mut candidates = Vec::new();
    let _ = shrink(&mut |candidate| {
        candidates.push(candidate);
        ControlFlow::Continue(())
    });
    candidates
}

/// Shrink `initial`. `candidates` proposes simpler values; `fails` runs the
/// property and reports whether the candidate fails the same way. Values are
/// compared by their full `Debug` rendering: seen before, or longer than the
/// current best, and a candidate is skipped.
pub(crate) fn shrink<T: Clone + Debug>(
    config: &ShrinkConfig,
    initial: T,
    mut candidates: impl FnMut(&T, &mut dyn FnMut(T) -> ControlFlow<()>) -> ControlFlow<()>,
    mut fails: impl FnMut(T) -> SameFailure,
) -> Shrunk<T> {
    let hasher = RandomState::new();
    let started = Instant::now();
    let mut stats = ShrinkStats::default();
    let exhausted = |stats: &ShrinkStats| config.exhausted(stats.steps, started);

    let mut current = initial;
    let current_debug = format!("{current:?}");
    let mut current_len = current_debug.len();
    let mut seen: HashSet<u64> = HashSet::new();
    seen.insert(hasher.hash_one(&current_debug));
    let mut failure: SameFailure = None;

    'outer: loop {
        if exhausted(&stats) {
            stats.budget_exhausted = true;
            break;
        }
        let mut accepted = None;
        let _ = candidates(&current, &mut |candidate| {
            if exhausted(&stats) {
                stats.budget_exhausted = true;
                return ControlFlow::Break(());
            }
            let debug = format!("{candidate:?}");
            let len = debug.len();
            if !seen.insert(hasher.hash_one(&debug)) {
                stats.rejected_seen += 1;
                return ControlFlow::Continue(());
            }
            if len > current_len {
                stats.rejected_not_smaller += 1;
                return ControlFlow::Continue(());
            }
            stats.steps += 1;
            if let Some(details) = fails(candidate.clone()) {
                stats.accepted += 1;
                accepted = Some((candidate, details, len));
                return ControlFlow::Break(());
            }
            ControlFlow::Continue(())
        });
        if let Some((candidate, details, len)) = accepted {
            failure = Some(details);
            current = candidate;
            current_len = len;
            continue 'outer;
        }
        break;
    }
    stats.duration = started.elapsed();
    Shrunk { value: current, failure, stats }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::{
            Gen,
            gens,
        },
    };

    #[test]
    fn descends_to_the_boundary() {
        let shrunk = shrink(
            &ShrinkConfig::default(),
            1000i64,
            |v, visitor| {
                visitor(v / 2)?;
                visitor(v - 1)
            },
            |v| (v >= 300).then_some((Some(format!("v={v}")), None)),
        );
        assert_eq!(shrunk.value, 300);
        assert_eq!(shrunk.failure.unwrap().0.as_deref(), Some("v=300"));
        assert!(shrunk.stats.accepted > 0);
        assert!(!shrunk.stats.budget_exhausted);
    }

    #[test]
    fn growing_candidates_terminate() {
        let shrunk = shrink(
            &ShrinkConfig::default(),
            5i64,
            |v, visitor| visitor(v + 1),
            |_| Some((None, None)),
        );
        // 6..9 are accepted (same length); 10 is longer and rejected.
        assert_eq!(shrunk.value, 9);
        assert_eq!(shrunk.stats.rejected_not_smaller, 1);
    }

    #[test]
    fn budget_and_seen_set() {
        let config = ShrinkConfig { max_steps: 2, ..ShrinkConfig::default() };
        let shrunk = shrink(
            &config,
            100i64,
            |v, visitor| {
                visitor(v - 1)?;
                visitor(v - 1)
            },
            |_| Some((None, None)),
        );
        assert_eq!(shrunk.value, 98);
        assert!(shrunk.stats.budget_exhausted);
        let none = shrink(
            &ShrinkConfig::default(),
            3i64,
            |_, _| ControlFlow::Continue(()),
            |_| Some((None, None)),
        );
        assert_eq!(none.value, 3);
        assert!(none.failure.is_none());
    }

    #[test]
    fn accepted_candidate_stops_production_immediately() {
        let mut produced = 0;
        let shrunk = shrink(
            &ShrinkConfig::default(),
            10i64,
            |value, visitor| {
                if *value == 10 {
                    for candidate in [5, 4, 3] {
                        produced += 1;
                        visitor(candidate)?;
                    }
                }
                ControlFlow::Continue(())
            },
            |value| (value == 5).then_some((None, None)),
        );

        assert_eq!(shrunk.value, 5);
        assert_eq!(produced, 1);
    }

    #[test]
    fn built_in_generators_shrink_to_golden_values() {
        let generator = gens::vec(gens::int(0i32..=1000));
        let fails = |v: &Vec<i32>| v.iter().sum::<i32>() > 100;
        let shrunk = shrink(
            &ShrinkConfig::default(),
            vec![400, 3, 250, 99],
            |v, visitor| generator.shrink(v, visitor),
            |v| fails(&v).then_some((None, None)),
        );
        assert_eq!(shrunk.value, vec![101]);

        let generator = gens::vec(gens::int(-1000i32..=1000));
        let unsorted = |v: &Vec<i32>| v.windows(2).any(|w| w[0] > w[1]);
        let shrunk = shrink(
            &ShrinkConfig::default(),
            vec![5, -3, 800, 2, 2],
            |v, visitor| generator.shrink(v, visitor),
            |v| unsorted(&v).then_some((None, None)),
        );
        assert_eq!(shrunk.value.len(), 2, "{:?}", shrunk.value);
        assert!(shrunk.value.iter().all(|x| x.abs() <= 1), "{:?}", shrunk.value);
    }

    #[test]
    fn long_values_are_compared_in_full() {
        // Renderings far past the report's truncation limit that differ only
        // at the end are different values, not ones already seen.
        let shrunk = shrink(
            &ShrinkConfig::default(),
            vec![1u8; 3000],
            |v, visitor| {
                if v.last() == Some(&1) {
                    let mut smaller = v.clone();
                    *smaller.last_mut().unwrap() = 0;
                    visitor(smaller)?;
                }
                ControlFlow::Continue(())
            },
            |_| Some((None, None)),
        );
        assert_eq!(shrunk.value.last(), Some(&0));
        assert_eq!(shrunk.stats.rejected_seen, 0);
    }
}
