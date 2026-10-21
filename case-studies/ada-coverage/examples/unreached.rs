//! Which ada functions the grammar reaches: generates cases, runs each on
//! both representations, and prints every ada function with the share of
//! its counters executed, least covered first.
//!
//! cargo run --release --example unreached -- [CASES]

use std::collections::BTreeSet;

use ada_coverage::{ada_counters, covered, is_ada};
use ephyra::{coverage::Functions, Gen, Source};
use ephyra_ada::{
    ada::{self, Kind},
    gen::Cases,
};

fn main() {
    let n: u64 = std::env::args().nth(1).map_or(20_000, |s| s.parse().unwrap());
    let ada = ada_counters();
    let mut hit = BTreeSet::new();
    for i in 0..n {
        let size = (i % 200) as usize;
        let case = Cases.generate(&mut Source::new(i, size));
        hit.extend(covered(&ada, || {
            ada::run(Kind::Url, &case, 0);
            ada::run(Kind::Aggregator, &case, 0);
        }));
    }
    println!("{} of {} ada counters hit by {n} cases", hit.len(), ada.len());
    let mut rows: Vec<(f64, usize, usize, String)> = Functions::get()
        .all()
        .iter()
        .filter(|f| is_ada(&f.name))
        .map(|f| {
            let total = f.counters.len();
            let got = f.counters.clone().filter(|c| hit.contains(c)).count();
            (
                got as f64 / total.max(1) as f64,
                got,
                total,
                f.name.rsplit(':').next().unwrap().to_owned(),
            )
        })
        .collect();
    rows.sort_by(|a, b| a.0.total_cmp(&b.0).then(b.2.cmp(&a.2)));
    for (share, got, total, name) in rows {
        println!("{:5.1}% {got:4}/{total:<4} {name}", share * 100.0);
    }
}
