// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Heritability of ephyra-sqlite's mutator: do mutants of an input that
//! reached a rare SQLite counter reach it again more often than fresh draws
//! do? Mutation-based fuzzing rests on that.
//!
//! Every input is exercised plainly (its case's file opened, its statements
//! run, each query run once), under a 5-second deadline, and its counters
//! are SQLite's (all of them, the file-format ones also counted apart).
//!
//! 1. `--fresh N` fresh draws (fixed seeds, sizes cycling 0..=100). A counter
//!    is rare when 1 to N/1000 of them reach it.
//! 2. Parents: fresh draws that reached a rare counter, up to `--parents P`
//!    of them, evenly spread. Each gets `--mutants M` mutants of each kind:
//!    `queries` (the queries mutated, the case kept), `case` (the case
//!    mutated, its queries repaired and kept within the row budget, as
//!    `CaseQueries` does), `both` (`CaseQueries`' own mutation), `stack-k`
//!    (`CaseQueries`' mutation applied k times in a row), and two splices
//!    with a donor (another fresh draw): `splice-queries` (the parent's case,
//!    the donor's queries) and `splice-case` (the donor's case, the parent's
//!    queries), the queries repaired against the case and kept within the
//!    row budget, or drawn anew when none is left.
//! 3. For each kind: retention, the share of mutants that reach a rare
//!    counter of their parent, against the share of fresh draws (the parent
//!    left out) that reach one of the same set; exploration, the share of
//!    mutants that reach a rare counter their parent did not, against the
//!    share of fresh draws that reach any rare counter; locality, the mean
//!    Jaccard similarity of a mutant's counters to its parent's, against
//!    that of fresh draws to the same parents; both, the share that keep a
//!    rare counter of their parent and reach a new one; time per mutant
//!    (mutation and exercise); and fallbacks and timeouts.
//!
//! `cargo run --release --bin heritability -- [--fresh N] [--parents P] [--mutants M] [--seed S]`

use std::{
    collections::{BTreeSet, HashMap},
    sync::atomic::Ordering,
    time::Instant,
};

use ephyra::{coverage, Gen, Source};
use ephyra_sqlite::{
    cases::Cases,
    expr::Queries,
    probes::{case_queries, final_schema},
    FILE_FALLBACKS, QUERY_FALLBACKS,
};
use sqlite_coverage::{exercise, format_counters, mix, queries_on, warm_up};

fn jaccard(a: &[u32], b: &[u32]) -> f64 {
    let (mut i, mut j, mut both) = (0, 0, 0usize);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                both += 1;
                i += 1;
                j += 1;
            },
        }
    }
    let union = a.len() + b.len() - both;
    if union == 0 {
        1.0
    } else {
        both as f64 / union as f64
    }
}

/// How a mutant is made.
#[derive(Clone, Copy)]
enum Kind {
    Queries,
    Case,
    Stack(u32),
    SpliceQueries,
    SpliceCase,
}

impl Kind {
    fn name(self) -> String {
        match self {
            Kind::Queries => "queries".into(),
            Kind::Case => "case".into(),
            Kind::Stack(1) => "both".into(),
            Kind::Stack(k) => format!("stack-{k}"),
            Kind::SpliceQueries => "splice-queries".into(),
            Kind::SpliceCase => "splice-case".into(),
        }
    }
}

#[derive(Default)]
struct Totals {
    mutants: u64,
    timeouts: u64,
    retained: u64,
    explored: u64,
    both: u64,
    secs: f64,
    jaccard: f64,
    file_fallbacks: u64,
    query_fallbacks: u64,
    // Per (parent, rare counter): mutants that reached it.
    pair_hits: u64,
    pairs: u64,
}

fn main() {
    coverage::suppress_default_dump();
    let args: Vec<String> = std::env::args().collect();
    let value = |flag: &str, default: u64| args.iter().position(|a| a == flag).map_or(default, |i| args[i + 1].parse().expect("a number"));
    let (n, parents_max, m, seed) = (value("--fresh", 30_000), value("--parents", 400), value("--mutants", 20), value("--seed", 0));
    let start = Instant::now();
    let format: BTreeSet<u32> = format_counters().into_iter().map(|c| c as u32).collect();
    let gen = case_queries();

    // 1. Fresh draws and how often each counter is reached, after a warm-up
    // so that once-per-process code does not count as rare.
    warm_up();
    let mut fresh: Vec<(u64, Vec<u32>)> = Vec::new();
    let mut fresh_timeouts = 0;
    let mut count: HashMap<u32, u32> = HashMap::new();
    for i in 0..n {
        let s = mix(seed, i);
        let v = gen.generate(&mut Source::new(s, (i % 101) as usize));
        match exercise(&v) {
            Some(cs) => {
                for &c in &cs {
                    *count.entry(c).or_default() += 1;
                }
                fresh.push((i, cs));
            },
            None => fresh_timeouts += 1,
        }
    }
    let ran = fresh.len() as f64;
    let limit = (fresh.len() / 1000).max(1) as u32;
    let rare: BTreeSet<u32> = count.iter().filter(|&(_, &k)| k <= limit).map(|(&c, _)| c).collect();
    let reached_any_rare = fresh.iter().filter(|(_, cs)| cs.iter().any(|c| rare.contains(c))).count() as f64 / ran;
    println!(
        "fresh: {} ran, {fresh_timeouts} timed out; {} counters reached ({} file-format); {} rare (reached by 1 to {limit}, {} file-format); {:.1}% of fresh draws reach a rare counter; {:.0} s",
        fresh.len(),
        count.len(),
        count.keys().filter(|c| format.contains(c)).count(),
        rare.len(),
        rare.iter().filter(|c| format.contains(c)).count(),
        100.0 * reached_any_rare,
        start.elapsed().as_secs_f64()
    );

    // 2. Parents, evenly spread over the fresh draws that reached a rare counter.
    let candidates: Vec<usize> = (0..fresh.len()).filter(|&k| fresh[k].1.iter().any(|c| rare.contains(c))).collect();
    let step = (candidates.len() as f64 / parents_max as f64).max(1.0);
    let chosen: Vec<usize> = (0..parents_max.min(candidates.len() as u64)).map(|j| candidates[(j as f64 * step) as usize]).collect();

    let mut kinds = vec![Kind::Queries, Kind::Case];
    kinds.extend([1, 2, 4, 8, 16, 32].map(Kind::Stack));
    kinds.extend([Kind::SpliceQueries, Kind::SpliceCase]);
    let mut totals: Vec<Totals> = kinds.iter().map(|_| Totals::default()).collect();
    let (mut base_retained, mut base_jaccard, mut base_n) = (0f64, 0f64, 0f64);
    for (p, &k) in chosen.iter().enumerate() {
        let (i, ref pcs) = fresh[k];
        let parent = gen.generate(&mut Source::new(mix(seed, i), (i % 101) as usize));
        let prare: Vec<u32> = pcs.iter().copied().filter(|c| rare.contains(c)).collect();
        // The fresh baseline for this parent: other fresh draws reaching one of
        // its rare counters, and their similarity to it (a sample of 300).
        let others: Vec<&Vec<u32>> = fresh.iter().filter(|(j, _)| *j != i).map(|(_, cs)| cs).collect();
        base_retained += others.iter().filter(|cs| prare.iter().any(|c| cs.binary_search(c).is_ok())).count() as f64 / others.len() as f64;
        let stride = (others.len() / 300).max(1);
        let sample: Vec<&&Vec<u32>> = others.iter().step_by(stride).collect();
        base_jaccard += sample.iter().map(|cs| jaccard(pcs, cs)).sum::<f64>() / sample.len() as f64;
        base_n += 1.0;
        for (kind, t) in kinds.iter().zip(totals.iter_mut()) {
            for j in 0..m {
                let mut g = Source::new(mix(seed ^ 0x5eed, (p as u64) * 1000 + j), (i % 101) as usize);
                let (f0, q0) = (FILE_FALLBACKS.load(Ordering::Relaxed), QUERY_FALLBACKS.load(Ordering::Relaxed));
                let t0 = Instant::now();
                let donor = || {
                    let d = mix(seed ^ 0xd0, (p as u64) * 1000 + j) % n;
                    gen.generate(&mut Source::new(mix(seed, d), (d % 101) as usize))
                };
                let mutant = match *kind {
                    Kind::Queries => (parent.0.clone(), Queries(final_schema(&parent.0)).mutate(&parent.1, &mut g)),
                    Kind::Case => {
                        let c = Cases.mutate(&parent.0, &mut g);
                        queries_on(c, &parent.1, &mut g)
                    },
                    Kind::Stack(k) => (0..k).fold(parent.clone(), |v, _| gen.mutate(&v, &mut g)),
                    Kind::SpliceQueries => queries_on(parent.0.clone(), &donor().1, &mut g),
                    Kind::SpliceCase => queries_on(donor().0, &parent.1, &mut g),
                };
                t.file_fallbacks += FILE_FALLBACKS.load(Ordering::Relaxed) - f0;
                t.query_fallbacks += QUERY_FALLBACKS.load(Ordering::Relaxed) - q0;
                t.mutants += 1;
                let exercised = exercise(&mutant);
                t.secs += t0.elapsed().as_secs_f64();
                let Some(cs) = exercised else {
                    t.timeouts += 1;
                    continue;
                };
                let hits: Vec<bool> = prare.iter().map(|c| cs.binary_search(c).is_ok()).collect();
                t.pair_hits += hits.iter().filter(|&&h| h).count() as u64;
                t.pairs += hits.len() as u64;
                t.retained += u64::from(hits.iter().any(|&h| h));
                let explored = cs.iter().any(|c| rare.contains(c) && pcs.binary_search(c).is_err());
                t.explored += u64::from(explored);
                t.both += u64::from(explored && hits.iter().any(|&h| h));
                t.jaccard += jaccard(pcs, &cs);
            }
        }
    }

    // 3. Report.
    let pair_base: f64 = {
        // The fresh rate of a parent's rare counter, averaged over (parent, counter) pairs.
        let (mut sum, mut k) = (0.0, 0.0);
        for &c in chosen.iter().flat_map(|&k| fresh[k].1.iter().filter(|c| rare.contains(c))) {
            sum += f64::from(count[&c]) / ran;
            k += 1.0;
        }
        sum / k
    };
    println!(
        "{} parents, {m} mutants of each kind each; fresh baseline: {:.2}% reach one of a parent's rare counters, {:.2}% reach any rare counter, a parent's rare counter is reached by {:.3}% of fresh draws, Jaccard to the parent {:.3}",
        chosen.len(),
        100.0 * base_retained / base_n,
        100.0 * reached_any_rare,
        100.0 * pair_base,
        base_jaccard / base_n
    );
    for (kind, t) in kinds.iter().zip(&totals) {
        let ok = (t.mutants - t.timeouts) as f64;
        let retention = t.retained as f64 / ok;
        println!(
            "{:14} retention {:5.1}% ({:6.1}x fresh); per rare counter {:5.1}% ({:5.0}x fresh); exploration {:5.2}% ({:4.2}x fresh); both {:5.2}%; Jaccard {:.3}; {:5.1} ms; fallbacks {} file, {} query; timeouts {}",
            kind.name(),
            100.0 * retention,
            retention / (base_retained / base_n),
            100.0 * t.pair_hits as f64 / t.pairs as f64,
            (t.pair_hits as f64 / t.pairs as f64) / pair_base,
            100.0 * t.explored as f64 / ok,
            (t.explored as f64 / ok) / reached_any_rare,
            100.0 * t.both as f64 / ok,
            t.jaccard / ok,
            1000.0 * t.secs / t.mutants as f64,
            t.file_fallbacks,
            t.query_fallbacks,
            t.timeouts
        );
    }
    println!("{:.0} s", start.elapsed().as_secs_f64());
}
