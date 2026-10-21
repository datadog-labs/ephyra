//! What the e-graph gives for generated seeds: `cargo run --release
//! --example stats -- <seeds> [out dir]`. Invalid draws are written to the
//! out dir (one file each) for diagnosis.

use ephyra::{Gen, Source};
use ephyra_postgresql::{cases::final_model, expr::Schema, probes::case_queries};
use ephyra_sqleq::{fragment, variants};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: u64 = args[1].parse().unwrap();
    let out = args.get(2).map(std::path::PathBuf::from);
    if let Some(o) = &out {
        std::fs::create_dir_all(o).unwrap();
    }
    let (mut queries, mut seeds, mut variants_total, mut drawn, mut undec, mut invalid, mut nodes) = (0, 0, 0, 0, 0, 0, 0);
    let start = std::time::Instant::now();
    for s in 0..n {
        let (case, qs) = case_queries().generate(&mut Source::new(s, (s % 101) as usize));
        let schema = Schema::of(&final_model(&case));
        queries += qs.len();
        for q in qs.iter().filter_map(fragment::seed) {
            seeds += 1;
            let (vs, r) = variants::variants(&q, &schema, 6, 60, s);
            variants_total += vs.len();
            drawn += r.drawn;
            nodes += r.nodes;
            undec += r.undecodable.len();
            invalid += r.invalid.len();
            if let Some(o) = &out {
                for (k, sql) in r.invalid.iter().enumerate() {
                    std::fs::write(o.join(format!("invalid_s{s}_{seeds}_{k}.sql")), format!("{}\n-- seed:\n{}\n", sql, q.sql())).unwrap();
                }
                for (k, e) in r.undecodable.iter().enumerate() {
                    std::fs::write(o.join(format!("undecodable_s{s}_{seeds}_{k}.txt")), e).unwrap();
                }
            }
        }
    }
    println!(
        "{queries} queries, {seeds} seeds in the fragment; {:.1} variants per seed; {drawn} drawn, {undec} undecodable, {invalid} invalid; mean e-nodes {:.0}; {:.1}s",
        variants_total as f64 / seeds.max(1) as f64,
        nodes as f64 / seeds.max(1) as f64,
        start.elapsed().as_secs_f64()
    );
}
