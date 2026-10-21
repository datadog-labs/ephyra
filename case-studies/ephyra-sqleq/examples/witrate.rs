//! Witness rates per anti-rule, without sqleq:
//! `cargo run --release --example witrate -- <first seed> <count>`.

use std::collections::BTreeMap;

use ephyra_sqleq::{antirules, inputs, witness};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (first, count): (u64, u64) = (args[1].parse().unwrap(), args[2].parse().unwrap());
    let mut by: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for s in first..first + count {
        let input = inputs::input(s);
        for (q, _) in &input.seeds {
            let named = antirules::anti(q, &input.schema);
            let vs: Vec<_> = named.iter().map(|(_, v)| v.clone()).collect();
            let ws = witness::witnesses(&input.case, &input.model, q, &vs).unwrap_or_else(|_| vec![None; vs.len()]);
            for ((rule, _), w) in named.iter().zip(ws) {
                let e = by.entry(rule.clone()).or_default();
                e.0 += 1;
                e.1 += usize::from(w.is_some());
            }
        }
    }
    for (r, (n, w)) in by {
        println!("{r:22} {w:4} / {n:<4} witnessed");
    }
}
