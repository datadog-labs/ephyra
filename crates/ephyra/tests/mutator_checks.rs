// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The randomness of checked mutants: independent of how the value was
//! generated, and the same whenever that value is checked again.
use ephyra::{
    __check_generator,
    BoxGen,
    Config,
    GenExt,
    Report,
    Source,
    check_mutator,
    gens,
};

/// The public checker and the one `ephyra check` runs.
type Checker = fn(&Config, BoxGen<u32>) -> Report;
const CHECKERS: [Checker; 2] = [
    |config, generator| check_mutator(config, generator, in_domain),
    |config, generator| __check_generator(config, generator, in_domain, |_, _| Vec::new()),
];

fn in_domain(value: &u32) -> bool {
    *value < 1000
}

fn cfg(seed: u64) -> Config {
    Config::default().seed(seed).no_regressions()
}

/// Leaves the domain once in 100 mutations, whatever the value.
fn rarely(value: &u32, g: &mut Source) -> u32 {
    if g.index(100) == 0 { 1000 } else { *value }
}

fn message(report: &Report) -> &str {
    report.failure.as_ref().and_then(|f| f.message.as_deref()).unwrap_or_default()
}

#[test]
fn a_seeded_mutator_does_not_replay_the_generator() {
    let transfer = |g: &mut Source| {
        let from = g.index(4);
        (from, (from + 1 + g.index(3)) % 4)
    };
    for seed in 0..20 {
        // Redraws `from`, which must differ from `to`.
        let broken = transfer.with_mutate(|t, g| (g.index(4), t.1));
        let report = check_mutator(&cfg(seed), broken, |t| t.0 != t.1);
        assert!(message(&report).contains("outside the support"), "{report}");
    }
    let fresh = |g: &mut Source| g.int(0u64..);
    let report = check_mutator(&cfg(1).tests(100), fresh, |_| true);
    assert!(report.passed(), "{report}");
    assert!(!report.health.iter().any(|w| w.kind == "mutator_inert"), "{report}");
}

#[test]
fn a_rare_mutator_failure_shrinks_and_replays() {
    for check in CHECKERS {
        for seed in 0..20 {
            let generator = || gens::int(0u32..=999).with_mutate(rarely).boxed();
            let report = check(&cfg(seed), generator());
            let failure = report.failure.as_ref().unwrap_or_else(|| panic!("{report}"));
            assert!(!failure.flaky, "{report}");
            assert_eq!(failure.counterexample().debug, "0", "{report}");

            let replayed =
                check(&cfg(seed + 100).tests(0).replay(failure.replay.unwrap()), generator());
            let again = replayed.failure.as_ref().unwrap_or_else(|| panic!("{replayed}"));
            assert_eq!(again.original, failure.original);
            assert_eq!(again.counterexample(), failure.counterexample());
            assert_eq!(again.message, failure.message);
        }
    }
}

#[test]
fn equal_values_meet_different_mutants() {
    for check in CHECKERS {
        for seed in 0..20 {
            let report = check(&cfg(seed), gens::constant(7u32).with_mutate(rarely).boxed());
            assert!(message(&report).contains("mutant 1000"), "{report}");
        }
    }
}

#[test]
fn a_mutator_gets_the_same_size_on_replay() {
    // The largest size in `check_mutator`, the size of the value (here of
    // iteration 7) in `ephyra check`.
    for (check, size, iteration) in [(CHECKERS[0], 100, 0), (CHECKERS[1], 3, 7)] {
        let escapes = move |v: &u32, g: &mut Source| if g.size() == size { 1000 } else { *v };
        let generator = || gens::int(0u32..=999).with_mutate(escapes).boxed();
        let report = check(&cfg(1), generator());
        let failure = report.failure.as_ref().unwrap_or_else(|| panic!("{report}"));
        assert_eq!(failure.iteration, iteration, "{report}");
        let replayed = check(&cfg(2).tests(0).replay(failure.replay.unwrap()), generator());
        assert!(message(&replayed).contains("mutant 1000"), "{replayed}");
    }
}

#[test]
#[cfg(feature = "json")]
fn a_mutator_failure_is_remembered() {
    let dir = std::env::temp_dir().join(format!("ephyra-mutator-checks-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (check, name) in CHECKERS.into_iter().zip(["check_mutator", "check_generator"]) {
        let config = Config::default().name(name).regressions_dir(&dir);
        let generator = || gens::int(0u32..=999).with_mutate(rarely).boxed();
        let found = check(&config.clone().seed(1), generator());
        assert!(found.regressions.saved, "{found}");

        let again = check(&config.seed(2), generator());
        assert_eq!((again.regressions.replayed, again.regressions.stale), (1, 0), "{again}");
        assert_eq!((again.tests, message(&again)), (0, message(&found)), "{again}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
