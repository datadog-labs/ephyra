// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Remembered failures across runs: what one run saves, the next one tries
//! first.

#![cfg(feature = "json")]

use std::{
    collections::{
        HashMap,
        HashSet,
    },
    fmt,
    path::Path,
    sync::atomic::{
        AtomicUsize,
        Ordering,
    },
};

use ephyra::{
    Config,
    Gen,
    GenExt,
    Report,
    Run,
    Source,
    Status,
    check,
    gens,
    loops::{
        self,
        SeedPool,
        Validity,
    },
    target,
};

fn cfg(dir: &Path, name: &str) -> Config {
    Config::default().seed(1).tests(1).name(name).regressions_dir(dir)
}

fn lines(dir: &Path, name: &str) -> usize {
    let text = std::fs::read_to_string(dir.join(format!("{name}.jsonl"))).unwrap_or_default();
    text.lines().count()
}

/// Sets of `n` items. The chance that 17 regenerations all render in the
/// same order is negligible from five items on.
fn sets(n: usize) -> impl Gen<Value = HashSet<u64>> {
    gens::vec(gens::int(0u64..=u64::MAX)).len(n..=n).collect()
}

#[test]
fn a_saved_hash_collection_is_replayed_and_not_saved_again() {
    let dir = tempfile::tempdir().unwrap();
    // 200 items render in more characters than the report shows.
    for n in [1, 5, 20, 200] {
        let name = format!("set{n}");
        let first = check(&cfg(dir.path(), &name), sets(n), |_| false);
        assert!(first.regressions.saved, "{first}");
        for seed in 2..12 {
            let report = check(&cfg(dir.path(), &name).seed(seed), sets(n), |_| false);
            let found = (report.regressions.replayed, report.regressions.stale, report.iterations);
            assert_eq!(found, (1, 0, 0), "{report}");
            assert!(!report.regressions.saved);
        }
        assert_eq!(lines(dir.path(), &name), 1);
    }

    // A rare failure inside a map fails again from the file alone.
    let maps = || gens::vec((gens::int(0u16..=u16::MAX), gens::bool())).len(6..=6).collect();
    let no_small_key = |m: HashMap<u16, bool>| m.keys().all(|key| *key >= 4);
    let first = check(&cfg(dir.path(), "map").tests(20_000), maps(), no_small_key);
    assert!(first.regressions.saved, "{first}");
    let again = check(&cfg(dir.path(), "map").seed(2).tests(0), maps(), no_small_key);
    assert_eq!((again.status, again.iterations), (Status::Failed, 0), "{again}");
    // The seed that found it.
    assert!(again.to_string().contains("search: Config::default().seed(1)"), "{again}");
}

#[test]
fn a_hash_collection_of_other_items_is_still_stale() {
    let dir = tempfile::tempdir().unwrap();
    let items = |range: std::ops::Range<u32>| {
        move |_: &mut Source| -> HashSet<u32> { range.clone().collect() }
    };
    assert!(check(&cfg(dir.path(), "set"), items(10..30), |_| false).regressions.saved);
    let same = check(&cfg(dir.path(), "set"), items(10..30), |_| true);
    assert_eq!((same.regressions.replayed, same.regressions.stale), (1, 0), "{same}");
    assert!(same.health.iter().any(|w| w.message.contains("stored order")), "{same}");
    let other = check(&cfg(dir.path(), "set"), items(11..31), |_| true);
    assert_eq!((other.regressions.replayed, other.regressions.stale), (1, 1), "{other}");
    assert!(other.health.iter().any(|w| w.message.contains("no longer matches")), "{other}");
}

#[test]
fn the_stored_order_is_tested_when_a_regeneration_has_it() {
    let dir = tempfile::tempdir().unwrap();
    let turn = AtomicUsize::new(0);
    let pairs = |_: &mut Source| {
        match turn.fetch_add(1, Ordering::Relaxed) % 2 {
            0 => [1, 2],
            _ => [2, 1],
        }
    };
    assert!(check(&cfg(dir.path(), "pair"), &pairs, |p| p != [1, 2]).regressions.saved);
    turn.store(1, Ordering::Relaxed);
    let again = check(&cfg(dir.path(), "pair").tests(0), &pairs, |p| p != [1, 2]);
    assert_eq!((again.status, again.regressions.stale), (Status::Failed, 0), "{again}");
    assert_eq!(again.counterexample().unwrap().debug, "[1, 2]");
}

#[cfg(feature = "derive")]
fn attribute_dir() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("ephyra-unordered-{}", std::process::id()))
}

#[cfg(feature = "derive")]
#[ephyra::test(config = Config::default().regressions_dir(attribute_dir()))]
#[should_panic]
#[ignore = "the next test runs it twice"]
fn small_collections(set: HashSet<u16>, map: HashMap<u16, Option<bool>>) {
    assert!(set.len() < 5 || map.len() < 5);
}

#[cfg(feature = "derive")]
#[test]
fn the_attribute_replays_hash_collections() {
    let report = || {
        let panic = std::panic::catch_unwind(small_collections).expect_err("it fails");
        *panic.downcast::<String>().unwrap()
    };
    let first = report();
    assert!(first.contains("remembered in "), "{first}");
    let second = report();
    assert!(second.contains("regressions: 1 stored input(s) replayed\n"), "{second}");
    assert!(second.contains("after 0 passing tests"), "{second}");
    assert!(!second.contains("remembered in "), "{second}");
    let _ = std::fs::remove_dir_all(attribute_dir());
}

/// Eight bytes whose sum random testing hardly ever gets to [`heavy`], while
/// mutation by the score does.
fn bytes() -> impl Gen<Value = Vec<u8>> {
    gens::vec(gens::int(0u8..=255)).len(8..=8)
}

fn heavy(v: Vec<u8>) -> bool {
    let sum: u32 = v.iter().map(|&x| u32::from(x)).sum();
    target(f64::from(sum));
    sum < 1900
}

fn search(dir: &Path, name: &str) -> Config {
    let mut config = cfg(dir, name).tests(200);
    config.target.tests = 5_000;
    config
}

/// `report` is of a failure found by mutation and saved under `config`'s
/// name: it is all that the next run has to try, with another seed, and it
/// is not saved again.
fn assert_remembered(config: &Config, report: &Report) {
    let failure = report.failure.as_ref().expect("a failure");
    assert!(failure.replay.is_none() && failure.iteration > 0, "found by mutation: {report}");
    assert!(report.regressions.saved, "{report}");
    assert!(report.to_string().contains("remembered in "), "{report}");

    let before = lines(&config.regressions.dir, &report.name);
    let next = check(&config.clone().seed(99).tests(0), bytes(), heavy);
    assert_eq!((next.status, next.iterations), (Status::Failed, 0), "{next}");
    assert_eq!((next.regressions.replayed, next.regressions.stale), (1, 0), "{next}");
    let replayed = next.failure.as_ref().unwrap();
    assert_eq!(replayed.original, failure.original);
    assert_eq!((replayed.replay, replayed.seed), (None, report.seed));
    assert!(!next.regressions.saved);
    assert!(!next.health.iter().any(|w| w.kind == "regressions"), "{next}");
    assert_eq!(lines(&config.regressions.dir, &report.name), before);
    // The seed that found it, not that of the run that replayed it.
    assert!(next.to_string().contains("reproduce: EPHYRA_SEED=1 "), "{next}");
    assert!(next.to_string().contains("search: Config::default().seed(1)"), "{next}");
}

#[test]
fn a_failing_mutant_of_the_targeted_phase_is_remembered() {
    let dir = tempfile::tempdir().unwrap();
    let config = search(dir.path(), "targeted");
    assert_remembered(&config, &check(&config, bytes(), heavy));

    let config = search(dir.path(), "several").failures(3);
    let report = check(&config, bytes(), heavy);
    assert_remembered(&config.clone().failures(1), &report);
    let next = check(&config.seed(99).tests(0), bytes(), heavy);
    assert_eq!((next.failures.len(), next.regressions.stale), (1, 0), "{next}");
}

#[test]
fn a_failing_mutant_of_a_shipped_loop_is_remembered() {
    fn climb<S: SeedPool<Vec<u8>, Validity>>(config: &Config, mut pool: S) -> Report {
        let mut run = Run::new(config, bytes(), heavy);
        loops::target(&mut run, &mut pool, 20_000);
        run.finish()
    }
    let dir = tempfile::tempdir().unwrap();
    let config = search(dir.path(), "heap");
    assert_remembered(&config, &climb(&config, loops::Heap::new()));
    let config = search(dir.path(), "double_queue");
    assert_remembered(&config, &climb(&config, loops::DoubleQueue::new()));
}

/// Random tests, kept with `keep_always` or `keep`, and then a climb through
/// the run's pool.
fn climb_by_hand(config: &Config, fixture: Option<Vec<u8>>, always: bool) -> Report {
    let mut run = Run::new(config, bytes(), heavy);
    while run.wants_more() {
        let Some(input) = run.draw() else { continue };
        let tested = run.test(&input);
        if always {
            run.keep_always(input.value, tested.score().unwrap_or(0.0));
        } else {
            run.keep(input.value, &tested);
        }
    }
    if let Some(fixture) = fixture {
        run.keep_always(fixture, 1899.0);
    }
    for _ in 0..50_000 {
        let Some(parent) = run.pool_pick() else { break };
        let Some(child) = run.mutate(&parent) else { continue };
        let tested = run.test(&child);
        if tested.failed() {
            break;
        }
        run.keep(child.value, &tested);
    }
    run.finish()
}

#[test]
fn a_failing_mutant_of_a_loop_written_by_hand_is_remembered() {
    let dir = tempfile::tempdir().unwrap();
    let config = search(dir.path(), "keep");
    assert_remembered(&config, &climb_by_hand(&config, None, false));
    let config = search(dir.path(), "keep_always");
    assert_remembered(&config, &climb_by_hand(&config, None, true));
}

#[derive(Clone)]
struct Blob(Vec<u8>);

impl fmt::Debug for Blob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Blob({} bytes)", self.0.len())
    }
}

fn assert_not_saved(report: &Report) {
    assert!(report.failure.as_ref().is_some_and(|f| f.replay.is_none()), "{report}");
    assert!(!report.regressions.saved, "{report}");
    let warnings: Vec<_> = report.health.iter().filter(|w| w.kind == "regressions").collect();
    assert_eq!(warnings.len(), 1, "{report}");
    assert!(warnings[0].message.contains("could not be saved"), "{report}");
    assert!(warnings[0].message.contains(&format!("EPHYRA_SEED={}", report.seed)), "{report}");
}

#[test]
fn a_mutant_that_cannot_be_made_again_is_not_saved() {
    let dir = tempfile::tempdir().unwrap();
    // A seed that the run did not make.
    let fixture = vec![255, 255, 255, 255, 255, 255, 255, 114];
    let report = climb_by_hand(&search(dir.path(), "fixture"), Some(fixture), false);
    assert_not_saved(&report);

    // A mutator whose result does not depend on its input and source alone.
    let calls = AtomicUsize::new(0);
    let counting =
        gens::int(0usize..=0).with_mutate(move |v, _| v + calls.fetch_add(1, Ordering::Relaxed));
    let report = check(&search(dir.path(), "counting"), counting, |x| {
        target(x as f64);
        x < 20
    });
    assert_not_saved(&report);

    // A rendering that all values share: what is made again is another value.
    let blobs = bytes().iso(Blob, |blob: &Blob| blob.0.clone());
    assert_not_saved(&check(&search(dir.path(), "blob"), blobs, |blob: Blob| heavy(blob.0)));

    // None leaves a file, and a stored mutant whose mutator changed is stale.
    let config = search(dir.path(), "changed");
    let step = |by: u64| gens::int(0u64..=0).with_mutate(move |v, _| v + by);
    let small = |x: u64| {
        target(x as f64);
        x < 20
    };
    assert!(check(&config, step(3), small).regressions.saved);
    let next = check(&config.clone().tests(0), step(3), small);
    assert_eq!((next.regressions.replayed, next.regressions.stale), (1, 0), "{next}");
    let next = check(&config.tests(0), step(2), small);
    assert_eq!((next.regressions.replayed, next.regressions.stale), (1, 1), "{next}");
    assert!(next.health.iter().any(|w| w.message.contains("(mutations: 7)")), "{next}");
    for name in ["fixture", "counting", "blob"] {
        assert_eq!(lines(dir.path(), name), 0);
    }
}

#[test]
fn a_mutant_that_is_made_again_only_by_chance_is_not_saved() {
    let dir = tempfile::tempdir().unwrap();
    // Which item of the set a mutation changes depends on their order.
    let generator = || (gens::vec(gens::int(0u8..=20)).len(2..=2).collect(), bytes());
    let property = |(_, v): (HashSet<u8>, Vec<u8>)| heavy(v);
    for seed in 0..20 {
        let config = search(dir.path(), &format!("chance{seed}")).seed(seed);
        if check(&config, generator(), property).regressions.saved {
            for seed in 100..110 {
                let next = check(&config.clone().seed(seed).tests(0), generator(), property);
                assert_eq!(next.regressions.stale, 0, "{next}");
            }
        }
    }
}

#[cfg(all(ephyra_instrumented, feature = "coverage"))]
#[test]
fn a_failing_mutant_of_the_guided_search_is_remembered() {
    /// Each right byte executes one more line, which random draws reach to
    /// the end once in 4^8.
    #[rustfmt::skip]
    fn stairs(v: &[u8]) -> usize {
        if v[0] != 3 { return 0; }
        if v[1] != 1 { return 1; }
        if v[2] != 2 { return 2; }
        if v[3] != 0 { return 3; }
        if v[4] != 3 { return 4; }
        if v[5] != 1 { return 5; }
        if v[6] != 2 { return 6; }
        if v[7] != 2 { return 7; }
        8
    }
    let staircase = || gens::vec(gens::int(0u8..=3)).len(8..=8);
    let dir = tempfile::tempdir().unwrap();
    let mut config = cfg(dir.path(), "guided").tests(200);
    config.coverage.guided = true;
    config.target.tests = 50_000;
    config.target.energy = 1_000;
    let report = check(&config, staircase(), |v: Vec<u8>| stairs(&v) < 8);
    assert!(report.failure.as_ref().is_some_and(|f| f.replay.is_none()), "{report}");
    assert!(report.regressions.saved, "{report}");
    let next = check(&config.seed(99).tests(0), staircase(), |v: Vec<u8>| stairs(&v) < 8);
    assert_eq!((next.status, next.iterations), (Status::Failed, 0), "{next}");
}
