// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Print the input `case_queries` draws for a seed and size, as a SQL
//! script psql runs (the setup's COPY data inline), for triage.
//!
//! `cargo run --example dump -- <seed> [size]`

use ephyra::{Gen, Source};
use ephyra_postgresql::{probes::case_queries, setup::Step};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let seed: u64 = args.get(1).and_then(|s| s.parse().ok()).expect("a seed");
    let size: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or((seed % 100) as usize);
    let (case, queries) = case_queries().generate(&mut Source::new(seed, size));
    for step in case.setup.script() {
        match step {
            Step::Sql(sql) => println!("{sql};"),
            Step::Copy(sql, data) => println!("{sql};\n{data}\\."),
        }
    }
    for s in &case.statements {
        println!("{s};");
    }
    println!("-- queries");
    for q in &queries {
        println!("{};", q.sql());
    }
}
