// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! `one_of` and `frequency` keep each value inside one alternative when they
//! shrink and mutate it.

use std::{
    cell::Cell,
    fmt::Debug,
    ops::ControlFlow,
    sync::atomic::{
        AtomicBool,
        AtomicUsize,
        Ordering::Relaxed,
    },
};

use ephyra::{
    BoxGen,
    Config,
    Gen,
    GenExt,
    Source,
    Status,
    any,
    check,
    check_mutator,
    check_shrinker,
    gens,
    target,
};

fn cfg() -> Config {
    Config::default().seed(1).tests(500).no_regressions()
}

/// An address: version 4 with 4 bytes, or version 6 with 16 bytes.
fn addr() -> BoxGen<(u8, Vec<u8>)> {
    gens::one_of(vec![
        (gens::constant(4u8), gens::vec(any::<u8>()).len(4..=4)).boxed(),
        (gens::constant(6u8), gens::vec(any::<u8>()).len(16..=16)).boxed(),
    ])
    .boxed()
}

fn valid((version, bytes): &(u8, Vec<u8>)) -> bool {
    matches!((version, bytes.len()), (4, 4) | (6, 16))
}

#[test]
fn hooks_never_mix_two_alternatives() {
    let size = |(_, bytes): &(u8, Vec<u8>)| bytes.iter().map(|b| u64::from(*b)).sum();
    check_shrinker(&cfg(), addr(), valid, size).assert_passed();
    // Without a seed the mutants are not drawn like the values.
    check_mutator(&Config { seed: None, ..cfg() }, addr(), valid).assert_passed();
    check(&cfg(), addr(), |v| {
        target(v.1.iter().map(|b| f64::from(*b)).sum::<f64>());
        valid(&v)
    })
    .assert_passed();
}

#[test]
fn the_alternative_that_produced_a_value_mutates_it() {
    let (value, mut g) = ((4, vec![1, 2, 3, 4]), Source::new(7, 10));
    let near = (0..1000)
        .filter(|_| {
            let (version, bytes) = addr().mutate(&value, &mut g);
            version == 4 && bytes.iter().zip(&value.1).filter(|(a, b)| a != b).count() <= 2
        })
        .count();
    // Half of the draws pick version 4; the rest are fresh version 6 values.
    assert!((400..600).contains(&near), "{near}");
}

#[test]
fn a_claimed_value_shrinks_to_claimed_ones() {
    // `constant` cannot tell, so the second alternative offers candidates too.
    let choice = gens::frequency(vec![
        (9, (gens::pick_from(vec![4u8]), gens::vec(any::<u8>()).len(4..=4)).boxed()),
        (1, (gens::constant(6u8), gens::vec(any::<u8>()).len(1..=16)).boxed()),
    ]);
    let smallest =
        smallest(choice, |(_, bytes)| bytes.iter().map(|b| u32::from(*b)).sum::<u32>() < 300);
    assert!(smallest.starts_with("(4, [0, 0, "), "{smallest}");
}

/// The value that a failing check of `generator` reports.
fn smallest<T: Clone + Debug>(generator: impl Gen<Value = T>, holds: fn(T) -> bool) -> String {
    let report = check(&cfg(), generator, holds);
    report.counterexample().expect("fails").debug.clone()
}

#[test]
fn numbers_shrink_across_the_gap_to_another_range() {
    let limit = gens::constant(u64::MAX).boxed();
    let sizes = gens::frequency(vec![(9, limit), (1, gens::int(0..=100).boxed())]);
    assert_eq!(smallest(sizes, |x| x < 30), "30");
    let pair = |range| (gens::vec(any::<u8>()).len(0..=20), gens::int(range)).boxed();
    let pairs = gens::frequency(vec![(1, pair(0..=100)), (9, pair(5000..=6000))]);
    assert_eq!(smallest(pairs, |(_, x)| x < 30), "([], 30)");
}

#[test]
fn an_endless_shrinker_of_another_alternative_ends() {
    static STEPS: AtomicUsize = AtomicUsize::new(0);
    let far = gens::int(1000i32..2000).with_shrink(|v, visit| {
        let mut x = *v;
        while x != 1000 && STEPS.fetch_add(1, Relaxed) < 1_000_000 {
            x = x.wrapping_sub(1);
            visit(x)?;
        }
        ControlFlow::Continue(())
    });
    let either = gens::one_of(vec![far.boxed(), gens::int(0..=500).boxed()]);
    assert_eq!(either.shrink_candidates(&2).last(), Some(&1));
    assert!(STEPS.load(Relaxed) < 10_000);
}

/// An empty vector with index 0, or a vector and an index into it.
fn cursor() -> BoxGen<(Vec<u8>, usize)> {
    gens::one_of(vec![
        gens::constant((Vec::new(), 0)).boxed(),
        gens::dependent(gens::vec(any::<u8>()).len(1..=8), |v| {
            FOREIGN.fetch_or(v.is_empty(), Relaxed);
            gens::int(0..v.len())
        })
        .boxed(),
    ])
    .boxed()
}

/// Whether the closure in [`cursor`] was called with an empty vector.
static FOREIGN: AtomicBool = AtomicBool::new(false);

#[test]
fn dependent_alternatives_never_see_foreign_values() {
    check(&cfg(), cursor(), |v| {
        target(-(v.0.len() as f64));
        true
    })
    .assert_passed();
    let report = check(&cfg(), cursor(), |(v, _)| !v.is_empty());
    assert_eq!(report.status, Status::Failed, "{report}");
    assert_eq!(report.failure.unwrap().original.debug, "([], 0)");
    assert!(!FOREIGN.load(Relaxed));
}

#[derive(Clone, Debug, PartialEq)]
enum Op {
    Pop,
    Push(u8),
    Set(u8, bool),
}

/// Each `back` handles its own variant and panics on the others.
fn op() -> BoxGen<Op> {
    let push = |op: &Op| if let Op::Push(x) = op { *x } else { unreachable!("not a push") };
    let set = |op: &Op| if let Op::Set(i, b) = op { (*i, *b) } else { unreachable!("not a set") };
    gens::one_of(vec![
        gens::pick_from(vec![Op::Pop]).boxed(),
        gens::int(0u8..=99).iso(Op::Push, push).boxed(),
        (gens::int(0u8..=9), gens::bool()).iso(|(i, b)| Op::Set(i, b), set).boxed(),
    ])
    .boxed()
}

#[test]
fn iso_alternatives_keep_to_their_variant() {
    let report = check(&cfg(), gens::vec(op()), |ops: Vec<Op>| {
        target(ops.len() as f64);
        !ops.iter().any(|op| matches!(op, Op::Push(10..)))
    });
    let failure = report.failure.expect("fails");
    assert_eq!(failure.shrunk.unwrap_or(failure.original).debug, "[Push(10)]");
    let mut g = Source::new(3, 10);
    let pushes = (0..1000).map(|_| op().mutate(&Op::Push(50), &mut g));
    let near = pushes.filter(|op| matches!(op, Op::Push(48..=52))).count();
    assert!(near > 100, "a third of the mutants change the push by a small step: {near}");
}

#[test]
fn opaque_alternatives_still_shrink_and_a_panic_in_one_is_skipped() {
    let halve = |v: &u32, visit: &mut dyn FnMut(u32) -> ControlFlow<()>| {
        assert!(*v < 1000, "not a small value");
        if *v > 0 { visit(v / 2) } else { ControlFlow::Continue(()) }
    };
    let decrement = |v: &u32, visit: &mut dyn FnMut(u32) -> ControlFlow<()>| visit(v - 1);
    let small = (|g: &mut Source| g.int(0u32..1000)).with_shrink(halve);
    let large = (|g: &mut Source| g.int(1000u32..5000)).with_shrink(decrement);
    let either = gens::one_of(vec![small.boxed(), large.boxed()]);
    assert_eq!(either.shrink_candidates(&3000), vec![2999]);
    assert_eq!(either.shrink_candidates(&8), vec![4, 7]);

    // A panic of the visitor is the caller's, whichever alternative runs, and
    // also through a choice inside a choice.
    let nested = gens::one_of(vec![either.boxed(), any::<u32>().map(|x| x).boxed()]);
    let visits = Cell::new(0);
    let visitor = std::panic::AssertUnwindSafe(|| {
        nested.shrink(&8, &mut |_| {
            visits.set(visits.get() + 1);
            panic!("from the visitor")
        })
    });
    let payload = std::panic::catch_unwind(visitor).unwrap_err();
    assert_eq!(payload.downcast_ref::<&str>(), Some(&"from the visitor"));
    assert_eq!(visits.get(), 1);
}

#[test]
fn alternatives_that_cannot_tell_regenerate() {
    let low = (|g: &mut Source| g.int(0u32..10)).with_mutate(|v, _| v + 1);
    let high = (|g: &mut Source| g.int(100u32..110)).with_mutate(|v, _| v + 1);
    let mut g = Source::new(5, 10);
    let opaque = gens::one_of(vec![low.boxed(), high.boxed()]);
    let supported = |v: u32| v < 10 || (100..110).contains(&v);
    assert!((0..200).all(|_| supported(opaque.mutate(&9, &mut g))));
    // Two copies of one generator are the same alternative.
    let next = (|g: &mut Source| g.int(0u32..10)).with_mutate(|v, _| v + 1).boxed();
    let twice = gens::one_of(vec![next.clone(), next]);
    assert!((0..20).all(|_| twice.mutate(&5, &mut g) == 6));

    /// Even numbers below 10, which a custom generator can tell.
    struct Even;
    impl Gen for Even {
        type Value = u32;
        fn generate(&self, g: &mut Source) -> u32 {
            g.int(0u32..5) * 2
        }
        fn mutate(&self, value: &u32, _g: &mut Source) -> u32 {
            (value + 2) % 10
        }
        fn can_produce(&self, value: &u32) -> Option<bool> {
            Some([0, 2, 4, 6, 8].contains(value))
        }
    }
    let told = gens::one_of(vec![Even.boxed(), gens::constant(99u32).boxed()]);
    assert!((0..200).all(|_| matches!(told.mutate(&6, &mut g), 8 | 99)));
    assert!((0..200).all(|_| matches!(told.mutate(&99, &mut g), 0 | 2 | 4 | 6 | 8 | 99)));
}
