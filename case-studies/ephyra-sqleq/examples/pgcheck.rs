//! Seeds and their variants on PostgreSQL with the case's data:
//! `cargo run --release --example pgcheck -- <first seed> <count> [out dir]`.
//! Differing pairs are written to the out dir.

use ephyra::{Gen, Source};
use ephyra_postgresql::{cases::final_model, expr::Schema, probes::case_queries};
use ephyra_sqleq::{fragment, pgcheck, variants};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let first: u64 = args[1].parse().unwrap();
    let count: u64 = args[2].parse().unwrap();
    let out = std::path::PathBuf::from(args.get(3).cloned().unwrap_or("/tmp/pgcheck".into()));
    std::fs::create_dir_all(&out).unwrap();
    let (mut compared, mut skipped, mut differ, mut seeds) = (0, 0, 0, 0);
    let start = std::time::Instant::now();
    for s in first..first + count {
        let (case, qs) = case_queries().generate(&mut Source::new(s, (s % 101) as usize));
        let schema = Schema::of(&final_model(&case));
        for (k, q) in qs.iter().filter_map(fragment::seed).enumerate() {
            seeds += 1;
            let (vs, _) = variants::variants(&q, &schema, 6, 60, s);
            match pgcheck::agree(&case, &q, &vs) {
                Ok(v) => {
                    compared += v.compared;
                    skipped += v.skipped;
                    for (j, (sql, why)) in v.differ.iter().enumerate() {
                        differ += 1;
                        std::fs::write(out.join(format!("s{s}_q{k}_{j}.txt")), format!("-- {why}\n{};\n{sql};\n", q.sql())).unwrap();
                    }
                },
                Err(e) => println!("seed {s}: {}", e.lines().next().unwrap_or("")),
            }
        }
    }
    println!("{seeds} seeds: {compared} variants agree, {differ} differ, {skipped} skipped; {:.1}s", start.elapsed().as_secs_f64());
}
