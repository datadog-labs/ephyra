// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Shrinking through `check`: the counterexample ends at the simplest value
//! in the order that the generator defines, also when that value prints
//! longer than the one it replaces.

use std::{
    fmt::Debug,
    time::Duration,
};

use ephyra::{
    Config,
    Gen,
    GenExt,
    any,
    check,
    gens,
};

/// Where `property` ends for each of a few seeds, after a few steps.
fn ends<G>(generator: impl Fn() -> G, property: impl Fn(G::Value) -> bool) -> Vec<String>
where
    G: Gen,
    G::Value: Clone + Debug,
{
    (0..20)
        .map(|seed| {
            let config = Config::default().seed(seed).no_regressions();
            let report = check(&config, generator(), &property);
            let failure = report.failure.expect("the property fails");
            let shrink = failure.shrink.as_ref().expect("it is shrunk");
            assert!(shrink.steps < 500, "{shrink:?}");
            failure.counterexample().debug.clone()
        })
        .collect()
}

#[test]
fn a_bool_ends_at_false() {
    assert_eq!(ends(gens::bool, |_| false), ["false"; 20]);
    let flags = || (gens::bool(), gens::vec(gens::bool()).len(2..=2), gens::int(0u8..=255));
    assert_eq!(ends(flags, |(_, _, n)| n < 10), ["(false, [false, false], 10)"; 20]);
}

#[test]
fn pick_from_ends_at_the_first_entry() {
    let picks = || gens::pick_from(vec!["default", "slow", "x"]);
    assert_eq!(ends(picks, |_| false), ["\"default\""; 20]);
    assert_eq!(ends(picks, |pick| pick == "default"), ["\"slow\""; 20]);
}

#[cfg(feature = "derive")]
#[test]
fn a_derived_enum_ends_at_the_first_variant() {
    #[derive(Clone, Debug, ephyra::Arbitrary)]
    enum Level {
        Default,
        Low,
    }

    #[derive(Clone, Debug, ephyra::Arbitrary)]
    struct Options {
        level: Level,
        verbose: bool,
        n: u8,
    }

    assert_eq!(
        ends(ephyra::any::<Options>, |o| o.n < 10),
        ["Options { level: Default, verbose: false, n: 10 }"; 20]
    );
}

#[test]
fn a_float_is_halved_whatever_its_digits() {
    let large = |x: f64| x.is_nan() || x <= 1000.0;
    for end in ends(gens::f64_bits, large).into_iter().chain(ends(any::<f64>, large)) {
        let x: f64 = end.parse().unwrap();
        assert!(x > 1000.0 && x / 2.0 <= 1000.0, "{end}");
    }
    let both = |(x, y): (f64, f64)| x.is_nan() || y.is_nan() || x.abs() <= 1e9 || y.abs() <= 1e9;
    ends(|| (gens::f64_bits(), gens::f64_bits()), both);
}

#[test]
fn a_float_is_not_halved_below_one() {
    assert_eq!(ends(|| gens::f64_range(0.0..=100.0), |x| x <= 0.0), ["1.0"; 20]);
    let positive = |x: f64| x.is_nan() || x <= 0.0;
    let narrow = ends(any::<f32>, |x| positive(x.into()));
    for end in ends(any::<f64>, positive).into_iter().chain(narrow) {
        // Nor does it turn into one of the special values.
        assert!(end.parse::<f64>().unwrap() <= 1.0, "{end}");
    }
}

#[test]
fn a_choice_of_ranges_ends_in_the_range_closest_to_zero() {
    let ports = || {
        gens::one_of(vec![
            gens::int(0u16..=1023).filter("system", |p| *p <= 1023).boxed(),
            gens::int(49152u16..=65535).filter("dynamic", |p| *p >= 49152).boxed(),
        ])
    };
    assert_eq!(ends(ports, |_| false), ["0"; 20]);
    let floats = || {
        gens::one_of(vec![
            gens::f64_range(0.0..=1.0).boxed(),
            gens::f64_range(100.0..=200.0).boxed(),
        ])
    };
    assert_eq!(ends(floats, |_| false), ["0.0"; 20]);
}

#[test]
fn an_attached_shrinker_is_followed() {
    // 500ms is the shorter time and the longer text.
    let second = || gens::constant(Duration::from_secs(1)).with_shrink(|d, visit| visit(*d / 2));
    assert_eq!(ends(second, |d| d < Duration::from_millis(250)), ["250ms"; 20]);
}
