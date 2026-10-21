//! How many one-step variants seeds get, by rule:
//! `cargo run --release --example onestep -- <seeds>`.

use std::collections::BTreeMap;

use ephyra_sqleq::{inputs, onestep};

fn main() {
    let n: u64 = std::env::args().nth(1).unwrap().parse().unwrap();
    let mut by_rule: BTreeMap<String, usize> = BTreeMap::new();
    let (mut seeds, mut total) = (0, 0);
    for s in 0..n {
        let input = inputs::input(s);
        for (q, _) in &input.seeds {
            seeds += 1;
            for (rule, _) in onestep::one_step(q, &input.schema) {
                *by_rule.entry(rule).or_default() += 1;
                total += 1;
            }
        }
    }
    println!("{seeds} seeds, {total} one-step variants");
    for (r, k) in by_rule {
        println!("  {r:22} {k}");
    }
}
