// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Showcase: equivalent SQL queries sampled from an e-graph.
//!
//! ```text
//! egg-demo <script.sql> [--n 8] [--samples 400] [--seed 1] [--out DIR]
//! egg-demo check <script.sql>... --out DIR [--n 8] [--samples 400] [--seed 1]
//!          [--sqleq-bin DIR] [--qed PATH]
//! ```
//!
//! The first form reads a schema and a query, saturates an e-graph with
//! sound rewrite rules, samples terms from it, keeps the most structurally
//! different, and checks each against the seed on random SQLite databases.
//! `check` runs the same pairs, and a mutant of each, past sqleq's frontend,
//! provers and disprover (see [`check`]).

use egg_demo::{check, pair_file, read, sqlite, variants};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str, default: u64| {
        (args.iter().position(|a| a == name)).and_then(|i| args.get(i + 1)).map_or(default, |v| v.parse().unwrap())
    };
    let (n, samples, seed) = (flag("--n", 8) as usize, flag("--samples", 400), flag("--seed", 1));
    if args.first().map(String::as_str) == Some("check") {
        std::process::exit(check::run(&args[1..], n, samples, seed));
    }
    let path = args.first().expect("usage: egg-demo <script.sql> [--n N] [--samples N] [--seed N] [--out DIR]");
    let input = read(path).unwrap_or_else(|e| {
        eprintln!("refused: {e}");
        std::process::exit(2);
    });
    println!("seed query\n  {}\n  as a term: {}\n", input.query, input.term);
    let (seed_sql, chosen) = variants(&input, n, samples, seed, true);
    for (k, (sql, term)) in chosen.iter().enumerate() {
        println!("{:>2}. ({} nodes)\n    {sql}\n", k + 1, term.as_ref().len());
    }
    let sqls: Vec<&String> = chosen.iter().map(|(s, _)| s).collect();
    // `--out DIR`: each (seed, variant) pair as an sqleq input file.
    if let Some(dir) = args.iter().position(|a| a == "--out").and_then(|i| args.get(i + 1)) {
        let stem = std::path::Path::new(path).file_stem().unwrap().to_string_lossy();
        std::fs::create_dir_all(dir).expect("the output directory is created");
        for (k, v) in sqls.iter().enumerate() {
            let file = format!("{dir}/{stem}_{:02}.sql", k + 1);
            std::fs::write(&file, pair_file(&input, path, v)).expect("the pair is written");
        }
        println!("wrote {} pairs to {dir}", sqls.len());
    }

    let trials = 300;
    let outcomes = sqlite::check(&input, &seed_sql, &sqls, trials, seed);
    if outcomes.iter().all(|(d, e)| *d == 0 && e.is_none()) {
        println!("checked on {trials} random SQLite databases: every variant returns the seed's rows on every one");
    } else {
        for (k, (d, e)) in outcomes.iter().enumerate().filter(|(_, (d, e))| *d > 0 || e.is_some()) {
            match e {
                // A variant that does not run is a broken rule or a printer bug, not a different answer.
                Some(e) => println!("variant {} does not run in SQLite: {e}", k + 1),
                None => println!("variant {} disagrees with the seed on {d} of {trials} databases", k + 1),
            }
        }
    }
}

