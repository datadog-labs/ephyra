// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! One property: **a saved failure is replayed, for its own property only.**
//!
//! A failing input is saved under the property's name, and the next run of that property
//! replays it before random testing. That is what makes a rare failure stay found. Two halves:
//!
//! - **The store** (`regressions::save` and `load`): after any sequence of saves under
//!   any names, and of the edits a file sees outside ephyra (a hand-deleted entry leaves no
//!   final newline; a stray non-UTF-8 line), `load(name)` returns exactly the entries saved
//!   under `name`, in order, one per point. Names that differ only in punctuation or in
//!   non-ASCII letters may share a file but never share entries.
//! - **End to end** (`check`): a failure saved by one run is replayed by the next run of the
//!   same property, not skipped as stale, for every kind of input. A run under another
//!   name replays nothing.
//!
//! See CONTRIBUTING.md, "Bugs: a property for the class, and a regression entry".
#![cfg(feature = "json")]

use std::{
    collections::{
        HashMap,
        HashSet,
    },
    fmt::Debug,
    fs,
    io::Write,
    path::PathBuf,
    sync::atomic::{
        AtomicU64,
        Ordering,
    },
};

use ephyra::{
    BoxGen,
    Config,
    Gen,
    GenExt,
    Source,
    TestResult,
    check,
    gens,
    regressions::{
        self,
        Entry,
    },
};

/// A fresh directory per case.
fn scratch(tag: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ephyra-replay-{tag}-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    dir
}

/// Property names from a small alphabet, so that names which map to the same file come up
/// often: punctuation, a space, non-ASCII letters.
fn names() -> impl Gen<Value = Vec<String>> {
    let ch = gens::pick_from(vec!['a', 'b', ':', '_', ' ', '-', '.', 'é', '名', '/']);
    gens::vec(gens::string_len(ch, 1..=5)).len(1..=4)
}

#[derive(Clone, Debug)]
enum Op {
    /// Save an entry under name `name % names.len()` at a point (seed, iteration).
    Save { name: usize, seed: u64, iteration: u64, debug: String },
    /// The file of a name loses its final newline, as when its last entry is deleted by hand.
    StripFinalNewline { name: usize },
    /// A line that is not UTF-8 is appended to the file of a name.
    AppendBadLine { name: usize },
}

fn ops() -> impl Gen<Value = Vec<Op>> {
    let op = gens::frequency(vec![
        (
            6,
            (gens::int(0usize..8), gens::int(0u64..3), gens::int(0u64..3), gens::int(0u16..1000))
                .map(|(name, seed, iteration, d)| {
                    Op::Save { name, seed, iteration, debug: format!("[{d}]") }
                })
                .boxed(),
        ),
        (1, gens::int(0usize..8).map(|name| Op::StripFinalNewline { name }).boxed()),
        (1, gens::int(0usize..8).map(|name| Op::AppendBadLine { name }).boxed()),
    ]);
    gens::vec(op).len(1..=24)
}

#[test]
fn a_saved_failure_is_replayed_for_its_own_property_only() {
    // The store, against a model: name -> entries in order, one per (seed, iteration).
    let config = Config::default().seed(48).tests(400).no_regressions();
    check(&config, (names(), ops()), |(names, ops)| {
        let dir = scratch("store");
        let mut model: HashMap<String, Vec<(u64, u64, String)>> = HashMap::new();
        for op in &ops {
            match op {
                Op::Save { name, seed, iteration, debug } => {
                    let name = &names[name % names.len()];
                    let entry = Entry {
                        name: name.clone(),
                        seed: *seed,
                        iteration: *iteration,
                        debug: debug.clone(),
                        shrunk: None,
                        created: 0,
                    };
                    regressions::save(&dir, &entry).unwrap();
                    let stored = model.entry(name.clone()).or_default();
                    if !stored.iter().any(|(s, i, _)| (s, i) == (seed, iteration)) {
                        stored.push((*seed, *iteration, debug.clone()));
                    }
                },
                Op::StripFinalNewline { name } => {
                    let path = regressions::path_for(&dir, &names[name % names.len()]);
                    if let Ok(bytes) = fs::read(&path) {
                        fs::write(&path, bytes.strip_suffix(b"\n").unwrap_or(&bytes)).unwrap();
                    }
                },
                Op::AppendBadLine { name } => {
                    let path = regressions::path_for(&dir, &names[name % names.len()]);
                    if path.exists() {
                        let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
                        f.write_all(b"\n{\"name\": \"\xff\"}\n").unwrap();
                    }
                },
            }
        }
        let mut problem = None;
        for name in names.iter().collect::<HashSet<_>>() {
            let loaded: Vec<_> = regressions::load(&dir, name)
                .into_iter()
                .map(|e| (e.seed, e.iteration, e.debug))
                .collect();
            let expected = model.get(name).cloned().unwrap_or_default();
            if loaded != expected {
                problem = Some(format!("load({name:?}) gave {loaded:?}, saved {expected:?}"));
                break;
            }
        }
        let _ = fs::remove_dir_all(&dir);
        problem.map_or(TestResult::Pass, TestResult::fail)
    })
    .assert_passed();

    // End to end: a failure one run saves, the next run replays. A plain loop over the
    // kinds of input and a few seeds, not a nested `check`: a `check` inside a property
    // deadlocks on instrumented builds (#61).
    let mut seeds = Source::new(46, 0);
    for (kind, replays) in INPUTS {
        for _ in 0..8 {
            let seed = seeds.int(0u64..=u64::MAX);
            if let Err(why) = replays(seed) {
                panic!("{kind}, seed {seed}: {why}");
            }
        }
    }
}

/// Run 1 of a property over `generator` that fails on every input saves the failure;
/// run 2 replays it (not stale); a run under a name sharing the file replays nothing.
fn replays<T: Clone + Debug + 'static>(generator: BoxGen<T>, seed: u64) -> Result<(), String> {
    let dir = scratch("e2e");
    let config = |name: &str| {
        Config::default().seed(seed).tests(20).max_size(8).name(name).regressions_dir(&dir)
    };
    let first = check(&config("props::fails"), generator.clone(), |_| false);
    let result = if !first.regressions.saved {
        Err(format!("run 1 did not save its failure: {:?}", first.regressions))
    } else {
        let again = check(&config("props::fails"), generator.clone(), |_| false);
        let other = check(&config("props_fails"), generator, |_| true);
        if (again.regressions.replayed, again.regressions.stale) != (1, 0) {
            Err(format!("run 2 replayed {:?}", again.regressions))
        } else if (other.regressions.replayed, other.regressions.stale) != (0, 0) {
            Err(format!("another property sharing the file replayed {:?}", other.regressions))
        } else {
            Ok(())
        }
    };
    let _ = fs::remove_dir_all(&dir);
    result
}

type Input = (&'static str, fn(u64) -> Result<(), String>);

/// Kinds of input, the ones whose `Debug` text is not a function of the value included.
const INPUTS: &[Input] = &[
    ("u64", |s| replays(gens::int(0u64..=u64::MAX).boxed(), s)),
    ("Vec<i8>", |s| replays(gens::vec(gens::int(-9i8..=9)).len(1..=6).boxed(), s)),
    ("String", |s| replays(gens::string_len(gens::char(), 1..=6).boxed(), s)),
    ("(bool, Option<u8>)", |s| {
        replays((gens::bool(), gens::option(gens::int(0u8..=9))).boxed(), s)
    }),
    ("HashSet<u16>", |s| {
        let set = gens::vec(gens::int(0u16..=999))
            .len(2..=6)
            .map(|v| v.into_iter().collect::<HashSet<_>>())
            .filter("two items", |s| s.len() >= 2);
        replays(set.boxed(), s)
    }),
    ("HashMap<u8, char>", |s| {
        let map = gens::vec((gens::int(0u8..=99), gens::ascii_char()))
            .len(2..=6)
            .map(|v| v.into_iter().collect::<HashMap<_, _>>())
            .filter("two items", |m| m.len() >= 2);
        replays(map.boxed(), s)
    }),
];
