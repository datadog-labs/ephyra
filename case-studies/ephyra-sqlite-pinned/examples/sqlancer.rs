// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! A driver for the oracle properties of `tests/sqlancer.rs` at scale,
//! without ephyra's per-test overhead: it draws inputs from the same
//! generator (`case_queries`) and runs exactly the same oracle functions
//! (`ephyra_sqlite::sqlancer`), seed after seed.
//!
//! `cargo run --release --example sqlancer -- <norec|tlp|plans|order|setops|cte|window|storage> <first seed> <count> [dump dir] [--queries LO-HI] [--mutate K] [--wal DIR]`
//!
//! Sizes cycle through 0..=100, as ephyra's runner draws them. With
//! `--queries LO-HI` each input is extended to k queries, k log-uniform in
//! the range by the seed, as the nightly runner's `gen` profiles extend
//! theirs (`--queries 1-300`: the runner's range; its `extend`, the same
//! sources); with `--mutate K` each input is then mutated K times by the
//! generator's own mutation (case and queries, statements and queries
//! repaired), as the runner's mutation profiles mutate a corpus input. A
//! plain burn draws fewer queries and never mutates: both found what it
//! missed (run 20261016-1828). A failure is
//! shrunk greedily with the generator's own shrinker (first candidate that
//! still fails, until none does), printed, and, with a dump directory, the
//! database after the case's statements is written there as a file the
//! sqlite3 CLI opens. A shrink candidate counts only when it fails with the
//! same first line of message. Ends with one summary line. The thread's
//! storage stays in memory (`storage` puts its WAL files in the system's
//! temporary directory) unless `--wal DIR` puts every case's database in a
//! WAL file in DIR (`sqlancer::Storage::Wal`), as the runner's `+wal`
//! profiles do; the last file is deleted at the end.

use std::{ops::ControlFlow, panic, time::Instant};

use ephyra::{Gen, Source};
use ephyra_sqlite::{
    cases::Case,
    expr::Query,
    probes::{case_queries, extend, mix},
    sqlancer::{self, Tally},
};

type Oracle = fn(&Case, &[Query]) -> Result<Tally, String>;

fn check(oracle: Oracle, v: &(Case, Vec<Query>)) -> Result<Tally, String> {
    match panic::catch_unwind(|| oracle(&v.0, &v.1)) {
        Ok(r) => r,
        Err(e) => Err(format!("panic: {:?}", e.downcast_ref::<String>().cloned().or(e.downcast_ref::<&str>().map(|s| s.to_string())))),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let oracle: Oracle = match args[1].as_str() {
        "norec" => sqlancer::norec,
        "tlp" => sqlancer::tlp,
        "plans" => sqlancer::plans,
        "order" => sqlancer::order,
        "setops" => sqlancer::setops,
        "cte" => sqlancer::ctes,
        "window" => sqlancer::windows,
        "storage" => sqlancer::storage,
        o => panic!("unknown oracle {o}"),
    };
    let first: u64 = args[2].parse().unwrap();
    let count: u64 = args[3].parse().unwrap();
    let flag = |name: &str| args.iter().position(|a| a == name).map(|i| args[i + 1].clone());
    let range = flag("--queries").map(|r| {
        let (lo, hi) = r.split_once('-').expect("--queries LO-HI");
        (lo.parse::<usize>().unwrap().max(1), hi.parse::<usize>().unwrap())
    });
    let mutations: u32 = flag("--mutate").map_or(0, |k| k.parse().unwrap());
    let dump = args.get(4).filter(|a| !a.starts_with("--")).cloned();
    if let Some(dir) = flag("--wal") {
        sqlancer::set_storage(sqlancer::Storage::Wal { dir: dir.into() });
    }
    panic::set_hook(Box::new(|_| {}));
    let gen = case_queries();
    let start = Instant::now();
    let (mut tally, mut failures, mut no_queries) = (Tally::default(), 0u64, 0u64);
    for i in 0..count {
        let seed = first + i;
        let size = (seed % 101) as usize;
        let mut g = Source::new(seed, size);
        let mut v = gen.generate(&mut g);
        if let Some(r) = range {
            extend(&mut v, seed, size, r);
        }
        if mutations > 0 {
            let mut g = Source::new(mix(seed, 10), size);
            for _ in 0..mutations {
                v = gen.mutate(&v, &mut g);
            }
        }
        no_queries += u64::from(v.1.is_empty());
        match check(oracle, &v) {
            Ok(t) => tally.add(&t),
            Err(e) => {
                failures += 1;
                if failures > 20 {
                    println!("FAIL seed {seed} (not shrunk): {}", e.lines().next().unwrap_or(""));
                    continue;
                }
                let mut best = v;
                // A candidate counts only if it fails the same way (the same
                // first line), so the failure does not slip into another one.
                let kind = e.lines().next().unwrap_or("").to_string();
                let mut why = e;
                for _ in 0..2000 {
                    let mut next = None;
                    let _ = gen.shrink(&best, &mut |c| match check(oracle, &c) {
                        Err(e) if e.lines().next().unwrap_or("") == kind => {
                            next = Some((c, e));
                            ControlFlow::Break(())
                        },
                        _ => ControlFlow::Continue(()),
                    });
                    match next {
                        Some((c, e)) => {
                            best = c;
                            why = e;
                        },
                        None => break,
                    }
                }
                println!("FAIL seed {seed}\n{why}\n{:?}\n{:?}", best.0, best.1);
                if let Some(dir) = &dump {
                    if let Ok(db) = sqlancer::open(&best.0) {
                        let path = format!("{dir}/{}-{seed}.db", args[1]);
                        let bytes = db.serialize(rusqlite::MAIN_DB).unwrap();
                        std::fs::write(&path, &*bytes).unwrap();
                        println!("database after the statements: {path}");
                    }
                }
            },
        }
    }
    sqlancer::clear_storage();
    println!(
        "SUMMARY {} seeds {first}..{} (queries {range:?}, {mutations} mutations): {count} cases, {failures} failures, {no_queries} without queries, {:?}, {:.1}s",
        args[1],
        first + count,
        tally,
        start.elapsed().as_secs_f64()
    );
}
