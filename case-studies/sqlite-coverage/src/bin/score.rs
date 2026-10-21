//! The score of ephyra-sqlite's case generator: the SQLite file-format code
//! that 10 cases are expected to reach beyond their controls.
//!
//! For each of N cases, drawn as `ephyra check` draws them (size
//! `floor(log2(i + 1))` for the i-th) from seeds fixed by `--seed`, the
//! counters its file executes under its statements, up to and including the
//! first VACUUM, but the control (the same rows, written by SQLite) does not. With `p_c` the share of cases that
//! reach counter c beyond their control, the score is
//! `Σ_c 1 − (1 − p_c)^10`: the counters a batch of 10 cases is expected to
//! reach. It rewards both what each case reaches and variety across cases,
//! and a counter one case in thousands reaches is worth almost nothing.
//!
//! `cargo run --release --bin score -- [--seed S] [--cases N]`

use std::{collections::BTreeMap, time::Instant};

use ephyra::{coverage, Gen, Source};
use ephyra_sqlite::cases::Cases;
use sqlite_coverage::{beyond, format_counters};

const BATCH: i32 = 10;

/// SplitMix64 of a seed and an index.
fn mix(seed: u64, index: u64) -> u64 {
    let mut z = seed.wrapping_add(index.wrapping_mul(0x9e37_79b9_7f4a_7c15));
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

fn main() {
    coverage::suppress_default_dump();
    let args: Vec<String> = std::env::args().collect();
    let value = |flag: &str, default: u64| {
        args.iter()
            .position(|a| a == flag)
            .map_or(default, |i| args[i + 1].parse().expect("a number"))
    };
    let (seed, n) = (value("--seed", 0), value("--cases", 10_000));
    let start = Instant::now();
    let format = format_counters();
    let mut reached: BTreeMap<usize, u64> = BTreeMap::new();
    for i in 0..n {
        let size = (u64::BITS - (i + 1).leading_zeros() - 1) as usize;
        let case = Cases.generate(&mut Source::new(mix(seed, i), size));
        for c in beyond(&format, &case) {
            *reached.entry(c).or_default() += 1;
        }
    }
    let score: f64 = reached
        .values()
        .map(|h| 1.0 - (1.0 - *h as f64 / n as f64).powi(BATCH))
        .sum();
    println!(
        "score {score:.1}, {n} cases, seed {seed}, {:.1} s",
        start.elapsed().as_secs_f64()
    );
}
