// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

use egg_demo::{parse, seed::{seeds, Seed}, sqlite, variants};

/// The generator and the parser agree on the fragment.
#[ephyra::test(gen = seeds())]
fn generated_seeds_parse(seed: Seed) {
    if let Err(e) = parse::read(&seed.to_string()) {
        panic!("refused: {e}");
    }
}

/// Every variant the e-graph samples for a seed returns the seed's rows on
/// random SQLite databases, and runs at all. A failure is a rule or printer
/// bug, shrunk to a small seed.
#[ephyra::test(gen = seeds(), config = ephyra::Config::default().tests(300))]
fn sampled_variants_agree_with_their_seed(seed: Seed) {
    let input = parse::read(&seed.to_string()).expect("the seed parses");
    let (seed_sql, chosen) = variants(&input, 6, 150, 1, false);
    let sqls: Vec<&String> = chosen.iter().map(|(s, _)| s).collect();
    for ((d, e), v) in sqlite::check(&input, &seed_sql, &sqls, 60, 1).iter().zip(&sqls) {
        assert!(e.is_none(), "a variant does not run: {}\n  {v}", e.as_deref().unwrap_or(""));
        assert_eq!(*d, 0, "a variant disagrees with the seed on {d} of 60 databases:\n  {v}");
    }
}
