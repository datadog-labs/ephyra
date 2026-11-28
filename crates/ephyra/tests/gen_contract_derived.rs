// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! One property: **a derived recursive type keeps the generator contract, within its
//! `max_depth`, and shrinks away the structure a failure does not need.**
//!
//! The catalogue covers the shapes `#[ephyra(max_depth = N)]` bounds: `Box<Self>`,
//! `Vec<Self>` and `Option<Box<Self>>` fields, in enums and structs, at depths 0 to 5, mixed,
//! and with a `support` predicate. For each type:
//!
//! 1. the generator contract of `gen_contract.rs` (replay; generate, a mutation chain and
//!    every shrink candidate inside the support; a shrink walk that settles; generation that
//!    spreads), with the support "no deeper than `N`";
//! 2. a failure that one leaf alone causes shrinks to that leaf: the reported counterexample
//!    has no structure left. Without a node's children among its candidates, a tree keeps
//!    its nodes.
//!
//! Types cannot be drawn at run time, so the catalogue is fixed; values, seeds and sizes
//! are drawn by ephyra. The second half runs `check` itself, so it is a plain loop rather
//! than a `check` over `check`, which would deadlock instrumented builds (#61).
//!
//! See CONTRIBUTING.md, "Bugs: a property for the class, and a regression entry".
#![cfg(feature = "derive")]

mod common;

use std::fmt::Debug;

use {
    common::keeps_contract,
    ephyra::{
        Arbitrary,
        Config,
        TestResult,
        any,
        check,
        gens,
    },
};

/// What the property needs to know about a recursive type.
trait Recursive: Arbitrary + Clone + Debug + PartialEq + Send + Sync + 'static {
    /// Its `max_depth`.
    const MAX: usize;
    fn depth(&self) -> usize;
    /// The numbers its leaves hold.
    fn numbers(&self) -> Vec<i64>;
    /// A valid value exactly `depth` deep.
    fn full(depth: usize) -> Self;
    /// The value with no structure that holds `n`.
    fn leaf(n: i64) -> Self;
}

#[derive(Debug, Clone, PartialEq, Arbitrary)]
#[ephyra(max_depth = 0)]
enum Flat {
    Leaf(u8),
    Node(Box<Flat>, Box<Flat>),
}

#[derive(Debug, Clone, PartialEq, Arbitrary)]
#[ephyra(max_depth = 1)]
enum Shallow {
    Leaf(u8),
    Node(Box<Shallow>, Box<Shallow>),
}

#[derive(Debug, Clone, PartialEq, Arbitrary)]
#[ephyra(max_depth = 4)]
enum Expr {
    Lit(i8),
    Neg(Box<Expr>),
    Add(Box<Expr>, Box<Expr>),
}

#[derive(Debug, Clone, PartialEq, Arbitrary)]
#[ephyra(max_depth = 2)]
struct Rose {
    value: u8,
    children: Vec<Rose>,
}

#[derive(Debug, Clone, PartialEq, Arbitrary)]
#[ephyra(max_depth = 5)]
struct Chain {
    value: i16,
    next: Option<Box<Chain>>,
}

#[derive(Debug, Clone, PartialEq, Arbitrary)]
#[ephyra(max_depth = 3)]
enum Mixed {
    Empty,
    Leaf(u8),
    Many(Vec<Mixed>),
    Pair { left: Box<Mixed>, right: Option<Box<Mixed>> },
}

fn all_even(e: &Even) -> bool {
    e.numbers().iter().all(|n| n % 2 == 0)
}

#[derive(Debug, Clone, PartialEq, Arbitrary)]
#[ephyra(max_depth = 3, support = all_even)]
enum Even {
    Lit(#[ephyra(gen = gens::int(0u8..=50).iso(|x| x * 2, |y: &u8| y / 2))] u8),
    Add(Box<Even>, Box<Even>),
}

/// The deepest of `depths`, plus one for the node itself, or 0 for a leaf.
fn below(depths: impl Iterator<Item = usize>) -> usize {
    depths.map(|d| d + 1).max().unwrap_or(0)
}

impl Recursive for Flat {
    const MAX: usize = 0;
    fn depth(&self) -> usize {
        match self {
            Flat::Leaf(_) => 0,
            Flat::Node(a, b) => below([a.depth(), b.depth()].into_iter()),
        }
    }
    fn numbers(&self) -> Vec<i64> {
        match self {
            Flat::Leaf(n) => vec![i64::from(*n)],
            Flat::Node(a, b) => [a.numbers(), b.numbers()].concat(),
        }
    }
    fn full(_: usize) -> Self {
        Flat::Leaf(1)
    }
    fn leaf(n: i64) -> Self {
        Flat::Leaf(n as u8)
    }
}

impl Recursive for Shallow {
    const MAX: usize = 1;
    fn depth(&self) -> usize {
        match self {
            Shallow::Leaf(_) => 0,
            Shallow::Node(a, b) => below([a.depth(), b.depth()].into_iter()),
        }
    }
    fn numbers(&self) -> Vec<i64> {
        match self {
            Shallow::Leaf(n) => vec![i64::from(*n)],
            Shallow::Node(a, b) => [a.numbers(), b.numbers()].concat(),
        }
    }
    fn full(depth: usize) -> Self {
        if depth == 0 {
            Shallow::Leaf(1)
        } else {
            Shallow::Node(Box::new(Self::full(depth - 1)), Box::new(Self::full(depth - 1)))
        }
    }
    fn leaf(n: i64) -> Self {
        Shallow::Leaf(n as u8)
    }
}

impl Recursive for Expr {
    const MAX: usize = 4;
    fn depth(&self) -> usize {
        match self {
            Expr::Lit(_) => 0,
            Expr::Neg(a) => below([a.depth()].into_iter()),
            Expr::Add(a, b) => below([a.depth(), b.depth()].into_iter()),
        }
    }
    fn numbers(&self) -> Vec<i64> {
        match self {
            Expr::Lit(n) => vec![i64::from(*n)],
            Expr::Neg(a) => a.numbers(),
            Expr::Add(a, b) => [a.numbers(), b.numbers()].concat(),
        }
    }
    fn full(depth: usize) -> Self {
        match depth {
            0 => Expr::Lit(1),
            1 => Expr::Neg(Box::new(Self::full(0))),
            _ => Expr::Add(Box::new(Self::full(depth - 1)), Box::new(Self::full(depth - 2))),
        }
    }
    fn leaf(n: i64) -> Self {
        Expr::Lit(n as i8)
    }
}

impl Recursive for Rose {
    const MAX: usize = 2;
    fn depth(&self) -> usize {
        below(self.children.iter().map(Rose::depth))
    }
    fn numbers(&self) -> Vec<i64> {
        let mut out = vec![i64::from(self.value)];
        out.extend(self.children.iter().flat_map(Rose::numbers));
        out
    }
    fn full(depth: usize) -> Self {
        let children = if depth == 0 { vec![] } else { vec![Self::full(depth - 1); 2] };
        Rose { value: 1, children }
    }
    fn leaf(n: i64) -> Self {
        Rose { value: n as u8, children: vec![] }
    }
}

impl Recursive for Chain {
    const MAX: usize = 5;
    fn depth(&self) -> usize {
        below(self.next.as_deref().map(Chain::depth).into_iter())
    }
    fn numbers(&self) -> Vec<i64> {
        let mut out = vec![i64::from(self.value)];
        out.extend(self.next.as_deref().map(Chain::numbers).unwrap_or_default());
        out
    }
    fn full(depth: usize) -> Self {
        let next = (depth > 0).then(|| Box::new(Self::full(depth - 1)));
        Chain { value: 1, next }
    }
    fn leaf(n: i64) -> Self {
        Chain { value: n as i16, next: None }
    }
}

impl Recursive for Mixed {
    const MAX: usize = 3;
    fn depth(&self) -> usize {
        match self {
            Mixed::Empty | Mixed::Leaf(_) => 0,
            Mixed::Many(children) => below(children.iter().map(Mixed::depth)),
            Mixed::Pair { left, right } => {
                below(std::iter::once(left.depth()).chain(right.as_deref().map(Mixed::depth)))
            },
        }
    }
    fn numbers(&self) -> Vec<i64> {
        match self {
            Mixed::Empty => vec![],
            Mixed::Leaf(n) => vec![i64::from(*n)],
            Mixed::Many(children) => children.iter().flat_map(Mixed::numbers).collect(),
            Mixed::Pair { left, right } => {
                let mut out = left.numbers();
                out.extend(right.as_deref().map(Mixed::numbers).unwrap_or_default());
                out
            },
        }
    }
    fn full(depth: usize) -> Self {
        match depth {
            0 => Mixed::Leaf(1),
            _ => {
                Mixed::Pair {
                    left: Box::new(Self::full(depth - 1)),
                    right: Some(Box::new(Mixed::Many(vec![]))),
                }
            },
        }
    }
    fn leaf(n: i64) -> Self {
        Mixed::Leaf(n as u8)
    }
}

impl Recursive for Even {
    const MAX: usize = 3;
    fn depth(&self) -> usize {
        match self {
            Even::Lit(_) => 0,
            Even::Add(a, b) => below([a.depth(), b.depth()].into_iter()),
        }
    }
    fn numbers(&self) -> Vec<i64> {
        match self {
            Even::Lit(n) => vec![i64::from(*n)],
            Even::Add(a, b) => [a.numbers(), b.numbers()].concat(),
        }
    }
    fn full(depth: usize) -> Self {
        if depth == 0 {
            Even::Lit(2)
        } else {
            Even::Add(Box::new(Self::full(depth - 1)), Box::new(Self::full(depth - 1)))
        }
    }
    fn leaf(n: i64) -> Self {
        Even::Lit(n as u8)
    }
}

/// Clause 1 for type `T`: the generator contract, with the support "no deeper than
/// `T::MAX`" (and, for `Even`, its own predicate).
fn keeps_its_contract<T: Recursive>(extra: fn(&T) -> bool, s: (u64, usize)) -> Result<(), String> {
    assert_eq!(T::full(T::MAX).depth(), T::MAX, "full() of {}", std::any::type_name::<T>());
    let support = move |v: &T| v.depth() <= T::MAX && extra(v);
    keeps_contract(&any::<T>(), support, &[T::full(T::MAX), T::leaf(0)], s)
}

/// Clause 2 for type `T`: a failure on any leaf holding `threshold` or more shrinks to
/// the bare leaf holding `threshold`.
fn shrinks_to_the_leaf<T: Recursive>(
    threshold: i64,
    seeds: std::ops::Range<u64>,
) -> Result<(), String> {
    let mut failed = 0;
    for seed in seeds {
        let config = Config::default().seed(seed).max_size(6).no_regressions();
        let report = check(&config, any::<T>(), |v: T| v.numbers().iter().all(|n| *n < threshold));
        let Some(failure) = &report.failure else { continue };
        failed += 1;
        let shown = &failure.counterexample().debug;
        let expected = format!("{:?}", T::leaf(threshold));
        if *shown != expected {
            return Err(format!(
                "seed {seed}: a failure on one leaf shrank to {shown}, not {expected} (found {})",
                failure.original.debug
            ));
        }
    }
    if failed == 0 {
        return Err("no seed failed: the clause tested nothing".to_string());
    }
    Ok(())
}

type Entry = (&'static str, fn((u64, usize)) -> Result<(), String>, fn() -> Result<(), String>);

const CATALOGUE: &[Entry] = &[
    (
        "Flat (max_depth 0)",
        |s| keeps_its_contract::<Flat>(|_| true, s),
        || shrinks_to_the_leaf::<Flat>(50, 0..8),
    ),
    (
        "Shallow (max_depth 1)",
        |s| keeps_its_contract::<Shallow>(|_| true, s),
        || shrinks_to_the_leaf::<Shallow>(50, 0..8),
    ),
    (
        "Expr (Box, max_depth 4)",
        |s| keeps_its_contract::<Expr>(|_| true, s),
        || shrinks_to_the_leaf::<Expr>(50, 0..8),
    ),
    (
        "Rose (Vec, max_depth 2)",
        |s| keeps_its_contract::<Rose>(|_| true, s),
        || shrinks_to_the_leaf::<Rose>(50, 0..8),
    ),
    (
        "Chain (Option<Box>, max_depth 5)",
        |s| keeps_its_contract::<Chain>(|_| true, s),
        || shrinks_to_the_leaf::<Chain>(50, 0..8),
    ),
    (
        "Mixed (all three, max_depth 3)",
        |s| keeps_its_contract::<Mixed>(|_| true, s),
        || shrinks_to_the_leaf::<Mixed>(50, 0..8),
    ),
    (
        "Even (with support, max_depth 3)",
        |s| keeps_its_contract::<Even>(all_even, s),
        || shrinks_to_the_leaf::<Even>(50, 0..8),
    ),
];

#[test]
fn every_derived_recursive_type_keeps_its_contract() {
    // 1: the contract, over types, seeds and sizes drawn by ephyra
    let config = Config::default().seed(143).tests(300).no_regressions();
    let case = (gens::int(0..CATALOGUE.len()), gens::int(0u64..=u64::MAX), gens::int(0usize..=6));
    check(&config, case, |(entry, seed, size)| {
        let (name, contract, _) = CATALOGUE[entry];
        match contract((seed, size)) {
            Ok(()) => TestResult::Pass,
            Err(why) => TestResult::fail(format!("{name}: {why}")),
        }
    })
    .assert_passed();
    // 2: shrinking, run after the check above, not inside it
    for (name, _, shrinks) in CATALOGUE {
        if let Err(why) = shrinks() {
            panic!("{name}: {why}");
        }
    }
}
