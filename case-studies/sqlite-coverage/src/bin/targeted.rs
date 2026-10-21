//! Targeted fuzzing of rare SQLite counters, after FairFuzz, against fresh
//! generation, at equal wall-clock time. Every input is an oracle input (a
//! case and its queries) exercised plainly ([`exercise`]).
//!
//! A frozen reference fixes what is rare, the same for every arm:
//! `--reference N` fresh draws from seeds of their own. A counter is rare
//! when 1 to N/1000 of them reach it, and new when none does. A run's score
//! is the rare and the new counters it reaches.
//!
//! - `gen`: fresh draws only.
//! - `target`: a fresh draw and a targeted mutant in turn. The pool holds,
//!   for each rare or new counter, the first [`PER_TARGET`] inputs that
//!   reached it. A mutant's target is the least reached in this run of 4
//!   counters drawn from the pool's, its parent one of the inputs that
//!   reached it; 70% of mutants are the parent's queries on a fresh donor's
//!   case (a fresh draw of a case alone, [`queries_on`]), the rest 4 of
//!   `CaseQueries`' mutations in a row.
//!
//! Every 30 seconds, and at the end, one line of `<out>/<arm>-<seed>.tsv`.
//!
//! `cargo run --release --bin targeted -- <gen|target> --seed S --minutes M --out DIR [--reference N]`

use std::{
    collections::{BTreeSet, HashMap},
    fmt::Write as _,
    fs::{self, File},
    io::Write as _,
    time::{Duration, Instant},
};

use ephyra::{coverage, Gen, Source};
use ephyra_sqlite::{
    cases::{Case, Cases},
    expr::Query,
    probes::case_queries,
    QUERY_FALLBACKS,
};
use sqlite_coverage::{exercise, mix, queries_on, warm_up};

type Input = (Case, Vec<Query>);

/// Inputs kept per target counter.
const PER_TARGET: usize = 8;

/// The seed the reference's draws are made from (none of the arms' seeds).
const REFERENCE_SEED: u64 = 0x005e_ed0f_0eef;

/// Seeds with a file over this many bytes are not kept.
const MAX_SEED_BYTES: usize = 2 << 20;

/// Totals for one kind of input (fresh or mutant).
#[derive(Default)]
struct Kind {
    inputs: u64,
    novel: u64,
    secs: f64,
}

fn main() {
    coverage::suppress_default_dump();
    let args: Vec<String> = std::env::args().collect();
    let target = match args[1].as_str() {
        "gen" => false,
        "target" => true,
        a => panic!("unknown arm {a}"),
    };
    let value = |flag: &str| args.iter().position(|a| a == flag).map(|i| args[i + 1].clone());
    let seed: u64 = value("--seed").map_or(0, |s| s.parse().unwrap());
    let minutes: f64 = value("--minutes").map_or(10.0, |s| s.parse().unwrap());
    let reference: u64 = value("--reference").map_or(30_000, |s| s.parse().unwrap());
    let out = value("--out").unwrap_or_else(|| ".".into());
    fs::create_dir_all(&out).unwrap();
    let mut tsv = File::create(format!("{out}/{}-{seed}.tsv", args[1])).unwrap();
    writeln!(tsv, "secs\texecs\trare\tnew\tcounters\tseeds\ttargets\ttimeouts\tquery_fallbacks\tfresh\tfresh_novel\tfresh_secs\tmutants\tmutant_novel\tmutant_secs").unwrap();

    // The reference: how many of its draws reach each counter, after a
    // warm-up so that once-per-process code does not count as rare.
    let gen = case_queries();
    warm_up();
    let mut count: HashMap<u32, u32> = HashMap::new();
    for i in 0..reference {
        let v = gen.generate(&mut Source::new(mix(REFERENCE_SEED, i), (i % 101) as usize));
        for c in exercise(&v).unwrap_or_default() {
            *count.entry(c).or_default() += 1;
        }
    }
    let limit = (reference / 1000).max(1) as u32;
    let rare_or_new = |c: u32| count.get(&c).is_none_or(|&k| k <= limit);
    let rare_total = count.values().filter(|&&k| k <= limit).count();
    writeln!(tsv, "# reference {reference} draws: {} counters, {rare_total} rare (reached by 1 to {limit})", count.len()).unwrap();

    // The run.
    let mut reached: HashMap<u32, u64> = HashMap::new(); // rare or new counter -> inputs that reached it
    let mut all: BTreeSet<u32> = BTreeSet::new();
    let mut seeds: Vec<Input> = Vec::new();
    let mut targets: HashMap<u32, Vec<usize>> = HashMap::new();
    let mut target_list: Vec<u32> = Vec::new();
    let (mut fresh, mut mutant) = (Kind::default(), Kind::default());
    let (mut execs, mut timeouts) = (0u64, 0u64);
    let mut rng = mix(seed, 0x7a);
    let mut next = move || {
        rng = mix(rng, 1);
        rng
    };
    let start = Instant::now();
    let deadline = start + Duration::from_secs_f64(minutes * 60.0);
    let mut next_line = start;
    loop {
        let now = Instant::now();
        let done = now >= deadline;
        if now >= next_line || done {
            let rare = reached.keys().filter(|c| count.contains_key(c)).count();
            let mut line = format!(
                "{:.0}\t{execs}\t{rare}\t{}\t{}\t{}\t{}\t{timeouts}\t{}",
                start.elapsed().as_secs_f64(),
                reached.len() - rare,
                all.len(),
                seeds.len(),
                target_list.len(),
                QUERY_FALLBACKS.load(std::sync::atomic::Ordering::Relaxed),
            );
            for k in [&fresh, &mutant] {
                write!(line, "\t{}\t{}\t{:.1}", k.inputs, k.novel, k.secs).unwrap();
            }
            writeln!(tsv, "{line}").unwrap();
            tsv.flush().unwrap();
            next_line = now + Duration::from_secs(30);
        }
        if done {
            break;
        }

        let t = Instant::now();
        let size = (execs % 101) as usize;
        let mutated = target && execs % 2 == 1 && !target_list.is_empty();
        let input = if mutated {
            // Of 4 targets drawn, the least reached; a parent that reached it.
            let c = (0..4)
                .map(|_| target_list[(next() % target_list.len() as u64) as usize])
                .min_by_key(|c| reached[c])
                .unwrap();
            let parents = &targets[&c];
            let parent = &seeds[parents[(next() % parents.len() as u64) as usize]];
            let mut g = Source::new(next(), size);
            if next() % 10 < 7 {
                let donor = Cases.generate(&mut Source::new(next(), size));
                queries_on(donor, &parent.1, &mut g)
            } else {
                (0..4).fold(parent.clone(), |v, _| gen.mutate(&v, &mut g))
            }
        } else {
            gen.generate(&mut Source::new(next(), size))
        };
        let exercised = exercise(&input);
        execs += 1;
        let k = if mutated { &mut mutant } else { &mut fresh };
        k.inputs += 1;
        k.secs += t.elapsed().as_secs_f64();
        let Some(cs) = exercised else {
            timeouts += 1;
            continue;
        };
        let mut novel = false;
        let mut keep_for = Vec::new();
        for &c in &cs {
            all.insert(c);
            if !rare_or_new(c) {
                continue;
            }
            let n = reached.entry(c).or_default();
            novel |= *n == 0;
            *n += 1;
            if targets.get(&c).is_none_or(|ps| ps.len() < PER_TARGET) {
                keep_for.push(c);
            }
        }
        if novel {
            k.novel += 1;
        }
        if target && !keep_for.is_empty() && input.0.file.0.len() <= MAX_SEED_BYTES {
            let at = seeds.len();
            seeds.push(input);
            for c in keep_for {
                let ps = targets.entry(c).or_default();
                if ps.is_empty() {
                    target_list.push(c);
                }
                ps.push(at);
            }
        }
    }
}
