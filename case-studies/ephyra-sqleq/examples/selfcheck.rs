// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! For the unproved one-step pairs of a run, whether sqleq proves the seed
//! equal to itself: `cargo run --release --example selfcheck -- <run dir>`.
//! Prints, per rule, unproved pairs whose seed is self-proved (the rule's
//! blind spot) and those whose seed is not (the context's).

use std::collections::{BTreeMap, BTreeSet};

use ephyra_sqleq::{check::{check_with, Pair}, ddl, inputs};

fn main() {
    let run = std::path::PathBuf::from(std::env::args().nth(1).unwrap());
    let tsv = std::fs::read_to_string(run.join("results.tsv")).unwrap();
    let unproved: Vec<String> = tsv.lines().skip(1).filter(|l| l.ends_with("\tunproved")).map(|l| l.split('\t').next().unwrap().to_string()).collect();
    // One self pair per seed query.
    let mut seeds: BTreeSet<(u64, usize)> = BTreeSet::new();
    for n in &unproved {
        let p: Vec<&str> = n.split('_').collect();
        seeds.insert((p[0][1..].parse().unwrap(), p[1][1..].parse().unwrap()));
    }
    let mut pairs = Vec::new();
    for &(s, k) in &seeds {
        let input = inputs::input(s);
        let q = &input.seeds[k].0;
        pairs.push(Pair { name: format!("s{s}_q{k}"), file_text: ddl::pair_file(&input.model, &q.sql(), &q.sql(), "self"), equivalent: true, pg_refutes: None });
    }
    let rows = check_with(pairs, &run.join("self"), 8, false);
    let proved: BTreeSet<String> = rows.iter().filter(|r| r.proved()).map(|r| r.name.clone()).collect();
    let mut by_rule: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut blind = String::new();
    for n in &unproved {
        let p: Vec<&str> = n.split('_').collect();
        let rule = p[2..p.len() - 1].join("_");
        let e = by_rule.entry(rule).or_default();
        if proved.contains(&format!("{}_{}", p[0], p[1])) {
            e.0 += 1;
            blind += &format!("{n}\n");
        } else {
            e.1 += 1;
        }
    }
    std::fs::write(run.join("rule-blind-spots.txt"), blind).unwrap();
    println!("{} seed queries; {} proved equal to themselves", seeds.len(), proved.len());
    println!("{:22} {:>10} {:>12}", "rule", "self-proved", "self-unproved");
    for (r, (a, b)) in by_rule {
        println!("{r:22} {a:>10} {b:>12}");
    }
}
