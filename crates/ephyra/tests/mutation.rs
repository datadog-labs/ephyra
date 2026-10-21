//! Regression coverage for value-returning mutators and their fallbacks.
use ephyra::{
    Config,
    Gen,
    GenExt,
    Run,
    Source,
    TestResult,
    gens,
};

#[test]
fn default_mutation_regenerates_non_clone_values() {
    struct NotClone(u64);
    let generator = |g: &mut Source| NotClone(g.int(0..=u64::MAX));
    let mut source = Source::new(17, 8);
    let mut expected = Source::new(17, 8);
    let child: NotClone = generator.mutate(&NotClone(0), &mut source);
    assert_eq!(child.0, generator.generate(&mut expected).0);

    let mapped = generator.map(|value| NotClone(value.0 / 2));
    let mut source = Source::new(17, 8);
    let mut expected = Source::new(17, 8);
    assert_eq!(mapped.mutate(&NotClone(0), &mut source).0, mapped.generate(&mut expected).0);
}

#[test]
fn attached_mutation_replaces_inner_even_when_unchanged() {
    let inner = gens::constant(5u8).with_mutate(|_, _| panic!("inner mutator must not run"));
    let generator = inner.with_mutate(|value, _| *value);
    assert_eq!(generator.mutate(&5, &mut Source::new(1, 0)), 5);
}

#[test]
fn singleton_and_empty_domains_return_values() {
    let mut source = Source::new(1, 0);
    assert_eq!(gens::constant(7).mutate(&7, &mut source), 7);
    assert_eq!(gens::pick_from(vec![7]).mutate(&7, &mut source), 7);
    assert_eq!(gens::int(7..=7).mutate(&7, &mut source), 7);
    let generators: [gens::IntGen<u8>; 0] = [];
    let empty: [u8; 0] = [];
    assert_eq!(generators.mutate(&empty, &mut source), empty);
    // Option<T> is the actual value, not a second "mutation unavailable" layer.
    let child: Option<u8> = gens::option(gens::constant(7)).mutate(&None, &mut source);
    assert_eq!(child, Some(7));
}

#[test]
fn filtered_mutation_regenerates_after_rejected_candidates() {
    let generator = gens::constant(2u8).with_mutate(|_, _| 3).filter("even", |v| v % 2 == 0);
    let mut source = Source::new(1, 0).with_filter_max_attempts(3);
    assert_eq!(generator.mutate(&2, &mut source), 2);
    assert!(source.discarded().is_none());
}

#[test]
fn runner_distinguishes_a_value_from_a_discard_or_panic() {
    let config = Config::default().seed(1).no_regressions();
    let mut run = Run::new(&config, gens::constant(2u8), |_| true);
    assert_eq!(run.mutate(&2).unwrap().value, 2);
    run.finish(); // Release the coverage lease before creating another run.

    let discarded = gens::constant(2u8).with_mutate(|_, source| {
        source.discard("mutation unavailable");
        0
    });
    let mut run =
        Run::new(&config, discarded, |_| -> bool { panic!("discarded value must not execute") });
    assert!(run.mutate(&2).is_none());
    let report = run.finish();
    assert_eq!(report.discarded, 1);
    assert_eq!(report.discard_reasons["mutation unavailable"], 1);

    let panicking = gens::constant(2u8).with_mutate(|_, _| panic!("mutation panic"));
    let mut run = Run::new(&config, panicking, |_| true);
    assert!(run.mutate(&2).is_none());
    assert_eq!(run.finish().abort.unwrap().message, "mutation panic");
}

#[test]
fn checks_reject_mutation_discards_even_when_the_value_is_valid() {
    let config = Config::default().seed(1).tests(32).no_regressions();
    let generator = || {
        gens::constant(2u8).with_mutate(|_, source| {
            source.discard("test discard");
            2
        })
    };
    let report = ephyra::check_mutator(&config, generator(), |value| *value == 2);
    assert!(!report.passed());
    let report =
        ephyra::__check_strategy(&config, generator(), |_| TestResult::Pass, |_, _| Vec::new());
    let message = report.failure.as_ref().and_then(|f| f.message.clone()).unwrap_or_default();
    assert!(message.contains("the mutator discarded"), "{report}");
}

#[cfg(feature = "derive")]
#[derive(Clone, Debug, PartialEq, ephyra::Arbitrary)]
struct Unit;

#[cfg(feature = "derive")]
#[derive(Clone, Debug, PartialEq, ephyra::Arbitrary)]
#[ephyra(support = |value: &Self| value.0 == 0)]
struct OnlyZero(#[ephyra(gen = gens::constant(0u8).with_mutate(|_, _| 1))] u8);

#[test]
#[cfg(feature = "derive")]
fn derived_mutation_returns_self_for_units_and_invalid_candidates() {
    let mut source = Source::new(1, 0);
    assert_eq!(ephyra::any::<Unit>().mutate(&Unit, &mut source), Unit);
    assert_eq!(ephyra::any::<OnlyZero>().mutate(&OnlyZero(0), &mut source), OnlyZero(0));
}

#[cfg(feature = "derive")]
#[derive(Clone, Debug, PartialEq, ephyra::Arbitrary)]
struct Wrapped(u32);

#[test]
#[cfg(feature = "derive")]
fn derived_single_field_mutation_spends_no_extra_draws() {
    // A one-field struct's mutation is its field's mutation, draw for draw.
    for seed in 0..50 {
        let derived = ephyra::any::<Wrapped>().mutate(&Wrapped(7), &mut Source::new(seed, 10));
        let field = ephyra::any::<u32>().mutate(&7, &mut Source::new(seed, 10));
        assert_eq!(derived, Wrapped(field), "seed {seed}");
    }
}
