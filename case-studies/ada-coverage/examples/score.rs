// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The joint coverage score of ephyra-ada's generators: the distinct ada
//! counters reached by running a fixed seed set from every generator
//! (URL cases on both representations, search params, URLPattern, pattern
//! lists, URLPatternInit inputs). Prints the total, the fresh-seed total
//! (the same count on a disjoint seed range, to catch changes that only fit
//! the fixed seeds) and a breakdown by area.
//!
//! cargo run --release --example score -- [SEEDS_PER_GENERATOR]

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use ada_coverage::{ada_counters, covered, is_ada};
use ephyra::{coverage::Functions, Gen, Source};
use ephyra_ada::{
    ada::{self, Kind},
    gen::Cases,
    init_inputs::{self, InitCases},
    params::{self, ParamsCases},
    pattern::{self, PatternCases},
    pattern_list::{self, ListCases},
};

/// Every counter reached by seeds `from..from + n` of every generator.
fn reach(ada: &BTreeSet<usize>, from: u64, n: u64) -> BTreeSet<usize> {
    let mut hit = BTreeSet::new();
    for i in from..from + n {
        let size = (i % 200) as usize;
        let case = Cases.generate(&mut Source::new(i, size));
        hit.extend(covered(ada, || {
            ada::run(Kind::Url, &case, 0);
            ada::run(Kind::Aggregator, &case, 0);
        }));
        let p = ParamsCases.generate(&mut Source::new(i, size));
        hit.extend(covered(ada, || {
            params::ada(&p);
        }));
        let u = PatternCases.generate(&mut Source::new(i, size % 100));
        hit.extend(covered(ada, || {
            pattern::ada(&u);
        }));
        let l = ListCases.generate(&mut Source::new(i, size));
        hit.extend(covered(ada, || {
            pattern_list::run(&l);
        }));
        let n = InitCases.generate(&mut Source::new(i, size % 100));
        hit.extend(covered(ada, || {
            init_inputs::ada(&n);
        }));
    }
    hit
}

/// The area of an ada function, from its demangled-ish mangled name.
fn area(name: &str) -> &'static str {
    let n = name.rsplit(':').next().unwrap_or(name);
    for (key, label) in [
        ("url_pattern_list", "pattern_list"),
        ("url_pattern", "url_pattern"),
        ("url_search_params", "search_params"),
        ("4idna", "idna"),
        ("url_aggregator", "url_aggregator"),
        ("3url", "url"),
        ("6parser", "parser"),
        ("7unicode", "unicode"),
        ("8checkers", "checkers"),
        ("7helpers", "helpers"),
    ] {
        if n.contains(key) {
            return label;
        }
    }
    "other"
}

fn main() {
    let n: u64 = std::env::args().nth(1).map_or(3000, |s| s.parse().unwrap());
    let ada = ada_counters();
    let t = Instant::now();
    let hit = reach(&ada, 0, n);
    let secs = t.elapsed().as_secs();
    let fresh = reach(&ada, 1_000_000, n);
    let mut by_area: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for f in Functions::get().all().iter().filter(|f| is_ada(&f.name)) {
        let e = by_area.entry(area(&f.name)).or_default();
        e.1 += f.counters.len();
        e.0 += f.counters.clone().filter(|c| hit.contains(c)).count();
    }
    println!(
        "score {} fresh {} of {} ada counters ({n} seeds per generator, {secs}s)",
        hit.len(),
        fresh.len(),
        ada.len()
    );
    // EA_GAPS=N: the N functions missing the most counters (mangled names;
    // pipe through c++filt).
    if let Some(k) = std::env::var("EA_GAPS").ok().and_then(|v| v.parse::<usize>().ok()) {
        let mut gaps: Vec<(usize, usize, String)> = Functions::get()
            .all()
            .iter()
            .filter(|f| is_ada(&f.name))
            .map(|f| {
                let missed = f.counters.clone().filter(|c| !hit.contains(c)).count();
                (missed, f.counters.len(), f.name.rsplit(':').next().unwrap_or(&f.name).to_owned())
            })
            .filter(|g| g.0 > 0)
            .collect();
        gaps.sort_by_key(|g| std::cmp::Reverse(g.0));
        for (missed, total, name) in gaps.into_iter().take(k) {
            println!("GAP {missed:4} of {total:<4} {name}");
        }
    }
    for (a, (got, total)) in by_area {
        println!(
            "  {a:<15} {got:5} / {total:<5} ({:.0}%)",
            100.0 * got as f64 / total.max(1) as f64
        );
    }
}
