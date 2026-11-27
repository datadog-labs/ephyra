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
    path::Path,
    sync::atomic::{
        AtomicUsize,
        Ordering,
    },
};

use ephyra::{
    Config,
    GenExt,
    Source,
    Status,
    check,
    gens,
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
fn sets(n: usize) -> impl ephyra::Gen<Value = HashSet<u64>> {
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
