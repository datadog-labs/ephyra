//! How often the generator reaches the type-affinity and collation gap
//! (`ephyra_sqlite::probes::gap_features`): the share of inputs holding each
//! feature, over inputs drawn as `sqlite-coverage`'s qscore draws them
//! (seeds `mix(seed, i)`, sizes cycling 0..=100).
//!
//! `cargo run --release --example gap_probe -- [--seed S] [--inputs N]`

use std::collections::BTreeMap;

use ephyra::{Gen, Source};
use ephyra_sqlite::probes::{case_queries, gap_features};

/// SplitMix64 of a seed and an index, as `sqlite_coverage::mix`.
fn mix(seed: u64, index: u64) -> u64 {
    let mut z = seed.wrapping_add(index.wrapping_mul(0x9e37_79b9_7f4a_7c15));
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let value = |flag: &str| args.iter().position(|a| a == flag).map(|i| args[i + 1].clone());
    let seed: u64 = value("--seed").map_or(0, |s| s.parse().unwrap());
    let n: u64 = value("--inputs").map_or(3000, |s| s.parse().unwrap());
    let gen = case_queries();
    let mut seen: BTreeMap<&str, u64> = BTreeMap::new();
    for i in 0..n {
        let v = gen.generate(&mut Source::new(mix(seed, i), (i % 101) as usize));
        for f in gap_features(&v.0, &v.1) {
            *seen.entry(f).or_default() += 1;
        }
    }
    println!("gap probe, {n} inputs, seed {seed}");
    for (f, k) in &seen {
        println!("  {f:24} {:6.2}%", 100.0 * *k as f64 / n as f64);
    }
}
