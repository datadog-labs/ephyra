//! `#[ephyra::generator]` both ways: predicates are kept as they are, and
//! generators get the check test that `ephyra check` runs.
#![cfg(feature = "derive")]
use ephyra::{
    Config,
    Gen,
    GenExt,
    gens,
};

#[ephyra::generator]
fn positive(value: &u8) -> bool {
    *value > 0
}

#[ephyra::generator]
fn sorted(values: &[u8]) -> bool {
    values.is_sorted()
}

#[ephyra::generator(positive, metrics = byte_metrics)]
fn positive_bytes() -> impl Gen<Value = u8> {
    gens::int(1u8..=255)
}

fn byte_metrics(value: &u8, _size: usize) -> Vec<(&'static str, f64)> {
    vec![("value", f64::from(*value))]
}

/// The predicate takes `&[u8]`; the check hands it a `&Vec<u8>`.
#[ephyra::generator(sorted)]
fn sorted_bytes() -> impl Gen<Value = Vec<u8>> {
    gens::vec(gens::int(0u8..=9)).map(|mut v| {
        v.sort();
        v
    })
}

/// Zero is generated, so the check fails.
#[ephyra::generator(positive)]
fn any_bytes() -> impl Gen<Value = u8> {
    gens::int(0u8..=255)
}

#[ephyra::generator]
fn unchecked_pairs() -> gens::BoxGen<(u8, u8)> {
    (gens::int(0u8..=9), gens::int(0u8..=9)).boxed()
}

#[test]
fn predicates_stay_callable() {
    assert!(positive(&1) && !positive(&0));
    assert!(sorted(&[1, 2]) && !sorted(&[2, 1]));
}

#[test]
fn generator_checks_pass_for_generators_that_keep_their_predicates() {
    __ephyra_check_positive_bytes();
    __ephyra_check_sorted_bytes();
    __ephyra_check_unchecked_pairs();
}

#[test]
fn generator_checks_fail_for_generators_that_leave_their_predicates() {
    assert!(std::panic::catch_unwind(__ephyra_check_any_bytes).is_err());
}

#[test]
fn a_panicking_predicate_is_a_violation() {
    let config = Config::default().seed(1).tests(50).no_regressions();
    let report = ephyra::__check_generator(
        &config,
        gens::constant(1u8),
        |_: &u8| -> bool { panic!("invalid predicate") },
        |_, _| Vec::new(),
    );
    let message = report.failure.as_ref().and_then(|f| f.message.clone()).unwrap_or_default();
    assert!(message.contains("the predicate panicked: invalid predicate"), "{report}");
}
