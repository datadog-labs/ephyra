// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Integration tests for `#[derive(Arbitrary)]` and `#[ephyra::test]`.
#![cfg(feature = "derive")]

use std::sync::atomic::{
    AtomicUsize,
    Ordering,
};

use ephyra::{
    Arbitrary,
    Config,
    Gen,
    Source,
    TestResult,
    any,
    check,
    check_mutator,
    check_shrinker,
    gens,
};

#[derive(Debug, Clone, PartialEq, Arbitrary)]
struct Point {
    x: i32,
    y: i32,
}

#[derive(Debug, Clone, Arbitrary)]
#[allow(dead_code)]
enum Shape {
    Dot,
    Line(Point, Point),
    Polygon { points: Vec<Point>, closed: bool },
}

#[derive(Debug, Clone, Arbitrary)]
struct Wrapper<T>(T);

#[derive(Debug, Clone, Arbitrary)]
struct Roll {
    #[ephyra(gen = gens::int(1u8..=6))]
    die: u8,
    #[ephyra(gen = gens::ascii_char())]
    tag: char,
}

fn not_dot(shape: &Guarded) -> bool {
    !matches!(shape, Guarded::Dot)
}

#[derive(Debug, Clone, PartialEq, Arbitrary)]
#[ephyra(support = not_dot)]
enum Guarded {
    Dot,
    Value(u8),
}

#[ephyra::shrinker]
fn shrink_steps(value: &u8) -> Vec<u8> {
    let mut candidates: Vec<u8> = Vec::with_capacity(2);
    if *value > 0 {
        candidates.push(0);
        candidates.extend(std::iter::once(*value - 1));
    }
    candidates
}

#[derive(Clone, Debug, PartialEq)]
enum ShrinkTree {
    Leaf,
    Node(Box<ShrinkTree>, Box<ShrinkTree>),
}

#[ephyra::shrinker]
fn shrink_tree(tree: &ShrinkTree) -> Vec<ShrinkTree> {
    match tree {
        ShrinkTree::Leaf => Vec::new(),
        ShrinkTree::Node(left, right) => {
            let mut candidates = vec![(**left).clone(), (**right).clone()];
            candidates.extend(
                shrink_tree(left)
                    .into_iter()
                    .map(|left| ShrinkTree::Node(Box::new(left), right.clone())),
            );
            candidates
        },
    }
}

static CAPACITY_EVALUATIONS: AtomicUsize = AtomicUsize::new(0);
static SHIPPED_SEARCH_CASES: AtomicUsize = AtomicUsize::new(0);
static CUSTOM_SEARCH_CASES: AtomicUsize = AtomicUsize::new(0);

fn capacity_hint() -> usize {
    CAPACITY_EVALUATIONS.fetch_add(1, Ordering::Relaxed);
    8
}

#[ephyra::shrinker]
fn shrink_iterator_pipeline(value: &u8) -> Vec<u8> {
    (0..*value)
        .filter(|candidate| candidate % 2 == 0)
        .flat_map(|candidate| [candidate, candidate.saturating_add(1)])
        .chain(std::iter::once(255))
        .collect()
}

#[ephyra::shrinker]
fn shrink_append_loop(value: &u8) -> Vec<u8> {
    let mut candidates = Vec::with_capacity(capacity_hint());
    for candidate in 0..*value {
        if candidate % 2 == 0 {
            candidates.push(candidate);
        }
    }
    candidates
}

#[ephyra::shrinker]
fn shrink_immutable_names(visitor: &u8, __ephyra_shrinker_visitor: &u8) -> Vec<u8> {
    let candidates = vec![*visitor, *__ephyra_shrinker_visitor];
    candidates
}

#[ephyra::test]
fn derived_types_generate(shape: Shape, wrapper: Wrapper<u8>, roll: Roll) -> bool {
    let _ = (&shape, wrapper.0);
    (1..=6).contains(&roll.die) && roll.tag.is_ascii()
}

#[ephyra::test(tests = 50, seed = 3)]
fn sorting_is_idempotent(mut v: Vec<i32>) -> bool {
    v.sort();
    let once = v.clone();
    v.sort();
    v == once
}

#[ephyra::test(tests = 20, seed = 9)]
fn relational_assumption_uses_the_whole_input(lo: u8, hi: u8) -> TestResult {
    ephyra::assume!(lo < hi);
    TestResult::check(lo < hi, "assumption admitted an unordered pair")
}

#[ephyra::test(tests = 50, gen = gens::int(1u32..=100))]
fn one_parameter_takes_a_strategy_of_its_own_type(n: u32) -> bool {
    (1..=100).contains(&n)
}

#[ephyra::test(tests = 50)]
fn parameters_take_strategies_of_their_own(
    count: u8,
    #[ephyra(gen = gens::string_len(gens::char(), 1..=8))] word: String,
) -> TestResult {
    ephyra::assume!(!word.is_empty());
    TestResult::check(
        word.repeat(usize::from(count)).len() == word.len() * usize::from(count),
        "repeat",
    )
}

/// A strategy for the whole input keeps the relation the property assumes.
fn ordered_pairs() -> impl Gen<Value = (u8, u8)> {
    gens::dependent(gens::int(0u8..=254), |lo| gens::int(*lo + 1..=255))
}

fn gap(pair: &(u8, u8), _size: usize) -> Vec<(&'static str, f64)> {
    vec![("gap", f64::from(pair.1 - pair.0))]
}

#[ephyra::test(tests = 50, gen = ordered_pairs(), metrics = gap)]
fn midpoints_lie_between(lo: u8, hi: u8) -> TestResult {
    ephyra::assume!(lo < hi);
    let mid = lo + (hi - lo) / 2;
    TestResult::check(lo <= mid && mid < hi, "the midpoint left the range")
}

#[test]
fn strategies_of_properties_that_can_discard_get_checks() {
    __ephyra_check_parameters_take_strategies_of_their_own();
    __ephyra_check_midpoints_lie_between();
}

#[ephyra::test(config = Config::default().tests(20).no_regressions())]
fn config_expression_is_accepted(a: u8, b: u8) -> Result<(), String> {
    if u16::from(a) + u16::from(b) == u16::from(b) + u16::from(a) {
        Ok(())
    } else {
        Err("not commutative".into())
    }
}

#[ephyra::test(
    config = Config::default().seed(4).tests(4).no_regressions(),
    search = |run| {
        SHIPPED_SEARCH_CASES.store(0, Ordering::Relaxed);
        let mut pool = ephyra::loops::Fifo::new();
        ephyra::loops::fuzz(run, &mut pool, 4);
        assert_eq!(SHIPPED_SEARCH_CASES.load(Ordering::Relaxed), 4);
    }
)]
fn shipped_search_loop_runs(value: u8) -> bool {
    SHIPPED_SEARCH_CASES.fetch_add(1, Ordering::Relaxed);
    let _ = value;
    true
}

#[ephyra::test(
    config = Config::default().seed(5).tests(3).no_regressions(),
    search = |run| {
        CUSTOM_SEARCH_CASES.store(0, Ordering::Relaxed);
        while run.wants_more() {
            let Some(input) = run.draw() else {
                continue;
            };
            run.test(&input);
        }
        assert_eq!(CUSTOM_SEARCH_CASES.load(Ordering::Relaxed), 3);
    }
)]
fn custom_search_loop_runs(value: u8) -> bool {
    CUSTOM_SEARCH_CASES.fetch_add(1, Ordering::Relaxed);
    let _ = value;
    true
}

#[test]
fn derived_struct_shrinks_field_by_field() {
    let config = Config::default().seed(1).tests(200).no_regressions();
    let report = check(&config, any::<Point>(), |p| p.x <= 10);
    assert_eq!(report.counterexample().unwrap().debug, "Point { x: 11, y: 0 }", "{report}");
    // any::<i32>() is a mix of edges and a uniform draw, so the candidates are the
    // union of both shrinkers: all closer to zero, zero first.
    let candidates = any::<Point>().shrink_candidates(&Point { x: 4, y: 0 });
    assert_eq!(candidates[0], Point { x: 0, y: 0 });
    assert!(
        candidates.contains(&Point { x: 2, y: 0 }) && candidates.contains(&Point { x: 3, y: 0 }),
        "{candidates:?}"
    );
    assert!(candidates.iter().all(|p| p.x.abs() < 4 && p.y == 0), "{candidates:?}");
}

#[test]
fn enum_payloads_shrink_within_the_variant() {
    let config = Config::default().seed(2).tests(200).no_regressions();
    let report = check(&config, any::<Shape>(), |s| matches!(s, Shape::Dot));
    let debug = &report.counterexample().unwrap().debug;
    assert!(
        debug == "Line(Point { x: 0, y: 0 }, Point { x: 0, y: 0 })"
            || debug == "Polygon { points: [], closed: false }",
        "{debug}"
    );
    // Earlier data-less variants are candidates for later ones.
    let candidates =
        any::<Shape>().shrink_candidates(&Shape::Line(Point { x: 0, y: 0 }, Point { x: 0, y: 0 }));
    assert!(matches!(candidates[0], Shape::Dot));
}

#[test]
fn support_predicate_filters_candidates_and_mutants() {
    let candidates = any::<Guarded>().shrink_candidates(&Guarded::Value(3));
    assert!(!candidates.contains(&Guarded::Dot), "{candidates:?}");
    assert!(candidates.contains(&Guarded::Value(0)));
    let mut g = Source::new(7, 5);
    for _ in 0..100 {
        let m = any::<Guarded>().mutate(&Guarded::Value(3), &mut g);
        assert!(not_dot(&m), "{m:?}");
    }
}

#[test]
fn eager_shrinker_macro_streams_and_stops_early() {
    let mut all = Vec::new();
    let flow = shrink_steps(&5, &mut |candidate| {
        all.push(candidate);
        std::ops::ControlFlow::Continue(())
    });
    assert_eq!(flow, std::ops::ControlFlow::Continue(()));
    assert_eq!(all, [0, 4]);

    let mut first = Vec::new();
    let flow = shrink_steps(&5, &mut |candidate| {
        first.push(candidate);
        std::ops::ControlFlow::Break(())
    });
    assert_eq!(flow, std::ops::ControlFlow::Break(()));
    assert_eq!(first, [0]);
}

#[test]
fn eager_shrinker_macro_composes_recursive_visitors() {
    let tree = ShrinkTree::Node(
        Box::new(ShrinkTree::Node(Box::new(ShrinkTree::Leaf), Box::new(ShrinkTree::Leaf))),
        Box::new(ShrinkTree::Leaf),
    );
    let mut candidates = Vec::new();
    let flow = shrink_tree(&tree, &mut |candidate| {
        candidates.push(candidate);
        std::ops::ControlFlow::Continue(())
    });
    assert_eq!(flow, std::ops::ControlFlow::Continue(()));
    assert_eq!(candidates.len(), 4);
}

#[test]
fn eager_shrinker_macro_handles_quickcheck_iterator_shapes() {
    let mut candidates = Vec::new();
    let flow = shrink_iterator_pipeline(&4, &mut |candidate| {
        candidates.push(candidate);
        std::ops::ControlFlow::Continue(())
    });
    assert_eq!(flow, std::ops::ControlFlow::Continue(()));
    assert_eq!(candidates, [0, 1, 2, 3, 255]);

    CAPACITY_EVALUATIONS.store(0, Ordering::Relaxed);
    candidates.clear();
    let flow = shrink_append_loop(&5, &mut |candidate| {
        candidates.push(candidate);
        std::ops::ControlFlow::Continue(())
    });
    assert_eq!(flow, std::ops::ControlFlow::Continue(()));
    assert_eq!(candidates, [0, 2, 4]);
    assert_eq!(CAPACITY_EVALUATIONS.load(Ordering::Relaxed), 1);
}

#[test]
fn eager_shrinker_macro_accepts_immutable_vecs_and_user_callback_names() {
    let mut candidates = Vec::new();
    let flow = shrink_immutable_names(&3, &5, &mut |candidate| {
        candidates.push(candidate);
        std::ops::ControlFlow::Continue(())
    });
    assert_eq!(flow, std::ops::ControlFlow::Continue(()));
    assert_eq!(candidates, [3, 5]);
}

#[test]
fn derived_hooks_pass_the_checkers() {
    let config = Config::default().seed(5).tests(200).no_regressions();
    let support = |r: &Roll| (1..=6).contains(&r.die) && r.tag.is_ascii();
    // Chars shrink toward 'a', so the size measure is the distance from 'a'.
    let size = |r: &Roll| u64::from(r.die) * 1000 + u64::from((r.tag as u32).abs_diff('a' as u32));
    check_shrinker(&config, any::<Roll>(), support, size).assert_passed();
    check_mutator(&config, any::<Roll>(), support).assert_passed();
    let point_support = |_: &Point| true;
    let point_size = |p: &Point| p.x.unsigned_abs() as u64 + p.y.unsigned_abs() as u64;
    check_shrinker(&config, any::<Point>(), point_support, point_size).assert_passed();
}

#[test]
fn every_variant_is_produced() {
    let mut source = Source::new(9, 10);
    let mut seen = [false; 3];
    for _ in 0..200 {
        match any::<Shape>().generate(&mut source) {
            Shape::Dot => seen[0] = true,
            Shape::Line(..) => seen[1] = true,
            Shape::Polygon { .. } => seen[2] = true,
        }
    }
    assert_eq!(seen, [true; 3]);
}
