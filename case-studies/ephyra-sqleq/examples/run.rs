//! The pipeline end to end: seeds from ephyra-postgresql, variants from the
//! e-graph and a mutant of each, PostgreSQL's verdict on the case's data,
//! then sqleq's tools (`ephyra_sqleq::check`).
//!
//! `cargo run --release --example run -- <first seed> <count> <out dir> [threads]`
//!
//! Writes `<out>/results.tsv`, the pairs under `<out>/pairs`, and prints the
//! cross-tab.

use ephyra::{Gen, Source};
use ephyra_postgresql::{cases::final_model, expr::Schema, probes::case_queries};
use ephyra_sqleq::{
    check::{self, Pair},
    ddl, fragment, mutate, pgcheck,
    sample::Rng,
    variants,
};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let first: u64 = args[1].parse().unwrap();
    let count: u64 = args[2].parse().unwrap();
    let out = std::path::PathBuf::from(&args[3]);
    let threads: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(8);
    // `--onestep`: one-step variants (at most two per rule and seed), named
    // `s{seed}_q{k}_{rule}_{j}`, instead of e-graph samples.
    let onestep = args.iter().any(|a| a == "--onestep");
    let start = std::time::Instant::now();
    let mut pairs = Vec::new();
    for s in first..first + count {
        let (case, qs) = case_queries().generate(&mut Source::new(s, (s % 101) as usize));
        let m = final_model(&case);
        let schema = Schema::of(&m);
        let mut rng = Rng::new(s ^ 0x6d75_7461);
        for (k, q) in qs.iter().filter_map(fragment::seed).enumerate() {
            let named: Vec<(String, _)> = if onestep {
                let mut per: std::collections::BTreeMap<String, usize> = Default::default();
                ephyra_sqleq::onestep::one_step(&q, &schema)
                    .into_iter()
                    .filter(|(r, _)| {
                        let n = per.entry(r.clone()).or_default();
                        *n += 1;
                        *n <= 2
                    })
                    .collect()
            } else {
                variants::variants(&q, &schema, 4, 60, s).0.into_iter().enumerate().map(|(j, v)| (format!("eq{j}"), v)).collect()
            };
            let vs: Vec<_> = named.iter().map(|(_, v)| v.clone()).collect();
            let mutants: Vec<_> = vs.iter().filter_map(|v| mutate::mutant(v, &mut rng)).filter(|m| m.valid(&schema) && m.sql() != q.sql()).collect();
            let judged = |list: &[_]| pgcheck::agree(&case, &q, list).ok();
            let pv = judged(&vs);
            let pm = judged(&mutants);
            let refuted = |verdict: &Option<pgcheck::Verdict>, sql: &str| verdict.as_ref().map(|v| v.differ.iter().any(|(d, _)| d == sql));
            let seed_sql = q.sql();
            for (j, (label, v)) in named.iter().enumerate() {
                let sql = v.sql();
                let name = if onestep { format!("s{s}_q{k}_{label}_{j}") } else { format!("s{s}_q{k}_{label}") };
                pairs.push(Pair { name, file_text: ddl::pair_file(&m, &seed_sql, &sql, &format!("ephyra-sqleq seed {s} query {k}: variant {j}")), equivalent: true, pg_refutes: refuted(&pv, &sql) });
            }
            for (j, mu) in mutants.iter().enumerate() {
                let sql = mu.sql();
                pairs.push(Pair { name: format!("s{s}_q{k}_mut{j}"), file_text: ddl::pair_file(&m, &seed_sql, &sql, &format!("ephyra-sqleq seed {s} query {k}: mutant {j}")), equivalent: false, pg_refutes: refuted(&pm, &sql) });
            }
        }
    }
    let generated = start.elapsed().as_secs_f64();
    let rows = check::check(pairs, &out, threads);
    std::fs::write(out.join("results.tsv"), check::tsv(&rows)).unwrap();
    print!("{}", check::report(&rows));
    println!("seeds {first}..{}: generation and PostgreSQL {generated:.1}s, sqleq {:.1}s", first + count, start.elapsed().as_secs_f64() - generated);
}
