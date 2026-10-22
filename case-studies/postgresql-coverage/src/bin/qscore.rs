// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The query-coverage score of ephyra-postgresql's oracle inputs: the
//! PostgreSQL backend code (`src/backend`) a batch of 10 inputs is expected
//! to reach under the oracles, on the instrumented server.
//!
//! After a warm-up, `--inputs N` fresh draws of `case_queries` (seeds fixed
//! by `--seed`, sizes cycling 0..=100) run in batches of 10, each input under
//! the oracle its index picks, each batch in one session (one backend), whose
//! reach is the counters that grew in the merged profile once its backend
//! exited. With `q_c` the share of batches that reach counter c, the score
//! is `Σ_c q_c`: the expected counters of a batch of 10, as
//! `sqlite-coverage`'s qscore estimates `Σ_c 1 − (1 − p_c)^10` from single
//! inputs. The oracles are the nine of `ephyra_postgresql::sqlancer` (since
//! 2026-10-16 with `views`: scores from before compare only as a different
//! measure). Also printed: the score by directory (the 20 largest), inputs per
//! second, failures (a PostgreSQL bug, or a generator or oracle mistake).
//!
//! The control (since 2026-10-18): before the measured batches,
//! [`CONTROL_BATCHES`] sessions that only open (a fresh database with its
//! extensions, the schema reset) and run `SELECT 1`. A counter that grows in
//! at least half of them is one any input reaches; `beyond control` sums
//! `q_c` over the other counters only, so connection and setup boilerplate
//! cannot raise it.
//!
//! `cargo run --release --bin qscore -- [--seed S] [--inputs N] [--dirs OUT.tsv]`

use std::{collections::BTreeMap, fs, panic, time::Instant};

use ephyra::{Gen, Source};
use ephyra_postgresql::{cases::Case, expr::Query, pg, probes::case_queries, sqlancer::{self, Tally}};
use postgresql_coverage::{function_files, use_instrumented_cluster, wait_exit, Cluster, Profile};

type Oracle = fn(&Case, &[Query]) -> Result<Tally, String>;

const ORACLES: [(&str, Oracle); 9] = [
    ("norec", sqlancer::norec),
    ("tlp", sqlancer::tlp),
    ("plans", sqlancer::plans),
    ("order", sqlancer::order),
    ("setops", sqlancer::setops),
    ("cte", sqlancer::ctes),
    ("window", sqlancer::windows),
    ("generic", sqlancer::generic),
    ("views", sqlancer::views),
];

const BATCH: u64 = 10;
const CONTROL_BATCHES: u64 = 20;

/// SplitMix64 of a seed and an index.
fn mix(seed: u64, index: u64) -> u64 {
    let mut z = seed.wrapping_add(index.wrapping_mul(0x9e37_79b9_7f4a_7c15));
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// The directory a source file is scored under: two levels below
/// `src/backend` (`optimizer/path`, `access/nbtree`, `executor`).
fn section(file: &str) -> Option<String> {
    let rest = file.strip_prefix("src/backend/")?;
    let parts: Vec<&str> = rest.split('/').collect();
    Some(match parts.len() {
        0 | 1 => "backend".into(),
        2 => parts[0].into(),
        _ => format!("{}/{}", parts[0], parts[1]),
    })
}

fn main() {
    panic::set_hook(Box::new(|_| {}));
    let args: Vec<String> = std::env::args().collect();
    let value = |flag: &str| args.iter().position(|a| a == flag).map(|i| args[i + 1].clone());
    let seed: u64 = value("--seed").map_or(0, |s| s.parse().unwrap());
    let n: u64 = value("--inputs").map_or(1000, |s| s.parse().unwrap());
    use_instrumented_cluster();
    let gen = case_queries();
    let run = |i: u64, seed: u64| {
        let input = gen.generate(&mut Source::new(mix(seed, i), (i % 101) as usize));
        let (name, oracle) = ORACLES[(i % ORACLES.len() as u64) as usize];
        (name, panic::catch_unwind(|| oracle(&input.0, &input.1)))
    };
    // Warm-up: the cluster started, caches filled, nothing counted.
    for i in 0..20 {
        let _ = run(i, 0x0077_a2a0);
    }
    let pid = pg::backend_pid();
    pg::close_session();
    if let Some(p) = pid {
        wait_exit(p);
    }
    let cluster = Cluster::find().unwrap();
    let profile = Profile::parse(&fs::read(&cluster.profile).unwrap()).unwrap();
    let sections: Vec<Option<String>> = function_files(&profile).unwrap().into_iter().map(|f| f.and_then(|f| section(&f))).collect();
    let in_scope = sections.iter().filter(|s| s.is_some()).count();
    // The control: sessions that do nothing but open.
    let mut control_hits: Vec<u64> = vec![0; sections.len()];
    let mut snap = cluster.snapshot().unwrap();
    for _ in 0..CONTROL_BATCHES {
        let _ = pg::with_session(|s| {
            s.fresh()?;
            s.execute("SELECT 1").map(|_| ())
        });
        let pid = pg::backend_pid();
        pg::close_session();
        if let Some(p) = pid {
            wait_exit(p);
        }
        let after = cluster.snapshot().unwrap();
        for (c, (x, y)) in snap.iter().zip(&after).enumerate() {
            if y > x {
                control_hits[c] += 1;
            }
        }
        snap = after;
    }
    let control: Vec<bool> = control_hits.iter().map(|&h| 2 * h >= CONTROL_BATCHES).collect();
    let start = Instant::now();
    let mut reached: Vec<u64> = vec![0; sections.len()];
    let (mut failures, mut comparisons, mut batches) = (0u64, 0usize, 0u64);
    let mut failed: BTreeMap<String, u64> = BTreeMap::new();
    let mut before = cluster.snapshot().unwrap();
    for b in 0..n / BATCH {
        for i in b * BATCH..(b + 1) * BATCH {
            match run(i, seed) {
                (_, Ok(Ok(t))) => comparisons += t.total(),
                (name, Ok(Err(e))) => {
                    failures += 1;
                    *failed.entry(format!("{name}: {}", e.lines().next().unwrap_or(""))).or_default() += 1;
                },
                (name, Err(_)) => {
                    failures += 1;
                    *failed.entry(format!("{name}: panic")).or_default() += 1;
                },
            }
        }
        let pid = pg::backend_pid();
        pg::close_session();
        if let Some(p) = pid {
            wait_exit(p);
        }
        let after = cluster.snapshot().unwrap();
        for (c, (x, y)) in before.iter().zip(&after).enumerate() {
            if y > x && sections[c].is_some() {
                reached[c] += 1;
            }
        }
        before = after;
        batches += 1;
    }
    let secs = start.elapsed().as_secs_f64();
    let score: f64 = reached.iter().map(|&h| h as f64 / batches as f64).sum();
    let beyond: f64 = reached.iter().zip(&control).filter(|(_, &k)| !k).map(|(&h, _)| h as f64 / batches as f64).sum();
    let in_control = control.iter().zip(&sections).filter(|(&k, s)| k && s.is_some()).count();
    let mut by_dir: BTreeMap<&str, (f64, usize)> = BTreeMap::new();
    for (c, &h) in reached.iter().enumerate() {
        if h > 0 {
            let e = by_dir.entry(sections[c].as_deref().unwrap()).or_default();
            e.0 += h as f64 / batches as f64;
            e.1 += 1;
        }
    }
    println!(
        "qscore {score:.1}, {} inputs in {batches} batches, seed {seed}, {secs:.1} s ({:.1} inputs/s), {} counters reached of {in_scope} in scope, {failures} failures, {comparisons} comparisons",
        batches * BATCH,
        (batches * BATCH) as f64 / secs,
        reached.iter().filter(|&&h| h > 0).count()
    );
    println!("beyond control {beyond:.1} ({in_control} backend counters in the control, {CONTROL_BATCHES} sessions that only open)");
    let mut dirs: Vec<_> = by_dir.iter().collect();
    dirs.sort_by(|a, b| b.1 .0.total_cmp(&a.1 .0));
    for (d, (s, k)) in dirs.iter().take(20) {
        println!("  {d:28} {s:9.1}  ({k} counters reached)");
    }
    for (why, k) in &failed {
        println!("  failure x{k}: {why}");
    }
    if let Some(out) = value("--dirs") {
        let mut total: BTreeMap<&str, usize> = BTreeMap::new();
        for s in sections.iter().flatten() {
            *total.entry(s.as_str()).or_default() += 1;
        }
        let text: String = total.iter().map(|(d, t)| {
            let (s, k) = by_dir.get(d).copied().unwrap_or((0.0, 0));
            format!("{d}\t{s:.2}\t{k}\t{t}\n")
        }).collect();
        fs::write(out, format!("dir\tscore\treached\ttotal\n{text}")).unwrap();
    }
}
