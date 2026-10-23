// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The relational rewrites on PostgreSQL with the case's data: every
//! `rel-..` one-step variant of each seed against the seed.
//! `cargo run --release --example relcheck -- <first seed> <count> <out dir>`;
//! differing pairs are written to the out dir.

use std::collections::BTreeMap;

use ephyra_sqleq::{inputs, pgcheck, relational};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (first, count): (u64, u64) = (args[1].parse().unwrap(), args[2].parse().unwrap());
    let out = std::path::PathBuf::from(&args[3]);
    std::fs::create_dir_all(&out).unwrap();
    let mut by_rule: BTreeMap<String, (usize, usize, usize)> = BTreeMap::new();
    for s in first..first + count {
        let input = inputs::input(s);
        for (k, (q, _)) in input.seeds.iter().enumerate() {
            for (j, (rule, v)) in relational::relational(q, &input.schema).into_iter().enumerate() {
                let e = by_rule.entry(rule.clone()).or_default();
                match pgcheck::agree(&input.case, q, std::slice::from_ref(&v)) {
                    Ok(verdict) if !verdict.differ.is_empty() => {
                        e.1 += 1;
                        let why = &verdict.differ[0].1;
                        std::fs::write(out.join(format!("s{s}_q{k}_{rule}_{j}.sql")), format!("-- {why}\n{};\n{};\n", q.sql(), v.sql())).unwrap();
                    },
                    Ok(verdict) if verdict.compared > 0 => e.0 += 1,
                    Ok(_) => e.2 += 1,
                    Err(err) => println!("seed {s}: {}", err.lines().next().unwrap_or("")),
                }
            }
        }
    }
    println!("{:24} {:>7} {:>7} {:>7}", "rule", "agree", "DIFFER", "skipped");
    for (r, (a, d, sk)) in by_rule {
        println!("{r:24} {a:>7} {d:>7} {sk:>7}");
    }
}
