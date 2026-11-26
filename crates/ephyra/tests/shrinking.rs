// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Shrinking through `check`: the counterexample ends at the simplest value
//! in the order that the generator defines, also when that value prints
//! longer than the one it replaces, and a panic on the way there loses
//! nothing.

use std::{
    fmt::Debug,
    ops::ControlFlow,
    time::Duration,
};

use ephyra::{
    Config,
    FailureInfo,
    Gen,
    GenExt,
    Report,
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

/// A failing run with panics in user code around the property: the report
/// is complete, and says where `kind` panicked.
fn assert_survives(report: &Report, kind: &str, message: &str) {
    assert!(report.failure.as_ref().is_some_and(|f| f.replay.is_some()), "{report}");
    let warnings: Vec<_> = report.health.iter().filter(|w| w.kind == kind).collect();
    assert_eq!(warnings.len(), 1, "{report}");
    assert!(warnings[0].message.contains("panicked at "), "{report}");
    assert!(warnings[0].message.contains(message), "{report}");
}

fn cfg() -> Config {
    Config::default().seed(1).no_regressions()
}

/// Panics on values below 10.
fn minus_ten(v: &u32, visit: &mut dyn FnMut(u32) -> ControlFlow<()>) -> ControlFlow<()> {
    visit(v.checked_sub(10).expect("ten or more"))
}

#[test]
fn a_panicking_shrinker_keeps_the_smallest_input_and_the_regression() {
    let generator = || gens::int(0u32..=1000).with_shrink(minus_ten);
    let report = check(&cfg(), generator(), |x| x % 10 != 7);
    assert_survives(&report, "shrinker_panicked", "ten or more");
    assert_eq!(report.counterexample().unwrap().debug, "7", "{report}");

    #[cfg(feature = "json")]
    {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::default().seed(1).name("panics").regressions_dir(dir.path());
        assert!(check(&config, generator(), |x| x % 10 != 7).regressions.saved);
        let again = check(&config.seed(2), generator(), |x| x % 10 != 7);
        assert_eq!((again.regressions.replayed, again.regressions.stale), (1, 0), "{again}");
        assert_eq!(again.tests, 0, "the saved failure is tried first");
    }

    // Each of the distinct failures survives.
    let report = check(&cfg().failures(3), generator(), |x| {
        assert!(x % 10 != 7, "seven");
        assert!(x % 10 != 3, "three");
    });
    assert_eq!(report.failures.len(), 2, "{report}");
    let mut ends: Vec<_> =
        report.failures.iter().map(|f| f.failure.counterexample().debug.as_str()).collect();
    ends.sort();
    assert_eq!(ends, ["3", "7"]);
}

#[test]
fn a_candidate_accepted_before_the_panic_is_kept() {
    let generator = gens::int(1u32..=20).with_shrink(|v, visit| {
        if *v == 0 {
            return ControlFlow::Continue(());
        }
        let _ = visit(v - 1);
        panic!("after the visit")
    });
    let report = check(&cfg(), generator, |_| false);
    assert_survives(&report, "shrinker_panicked", "after the visit");
    assert_eq!(report.counterexample().unwrap().debug, "0", "{report}");
}

#[test]
fn panics_in_combinators_during_shrinking_are_survived() {
    let evens = gens::int(0u32..=100).iso(|n| n * 2, |n| n.checked_sub(2).expect("not zero") / 2);
    let report = check(&cfg(), evens, |_| false);
    assert_survives(&report, "shrinker_panicked", "not zero");
    assert_eq!(report.counterexample().unwrap().debug, "0");

    let filtered = gens::int(1u32..=1000)
        .with_shrink(|v, visit| visit(v / 2))
        .filter("divides", |v| 1000 / v > 0);
    let report = check(&cfg(), filtered, |_| false);
    assert_survives(&report, "shrinker_panicked", "divide by zero");
    assert_eq!(report.counterexample().unwrap().debug, "1");
}

#[cfg(feature = "derive")]
#[test]
fn a_panic_in_a_derived_support_predicate_is_survived() {
    #[derive(Clone, Debug, ephyra::Arbitrary)]
    #[ephyra(support = Span::valid)]
    struct Span {
        #[ephyra(gen = gens::int(10u8..=100))]
        start: u8,
        #[ephyra(gen = gens::int(6u8..=10))]
        back: u8,
    }

    impl Span {
        fn valid(&self) -> bool {
            self.start.checked_sub(self.back + 5).expect("not before zero") < 200
        }
    }

    let report = check(&cfg(), ephyra::any::<Span>(), |s: Span| s.start < 20);
    assert_survives(&report, "shrinker_panicked", "not before zero");
}

#[test]
fn a_panicking_signature_function_falls_back_to_the_default_name() {
    let config = cfg()
        .failures(2)
        .signature(|f: &FailureInfo| f.location.as_ref().expect("a location").file.clone());
    let report = check(&config, gens::int(0u32..=1000), |x| {
        assert!(x < 900, "large");
        x < 10
    });
    assert_survives(&report, "signature_panicked", "a location");
    let mut names: Vec<_> = report.failures.iter().map(|f| f.signature.clone()).collect();
    names.sort();
    assert_eq!(names, [file!().replace('\\', "/"), "returned false".to_string()], "{report}");
}
