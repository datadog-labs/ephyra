//! How ephyra-postgresql's queries fare in sqleq's frontend, each paired
//! with itself: `cargo run --release --example probe -- <seeds> <out dir>`.

use std::collections::BTreeMap;

use ephyra::{Gen, Source};
use ephyra_postgresql::{cases::final_model, probes::case_queries};
use ephyra_sqleq::{ddl, tools};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: u64 = args[1].parse().unwrap();
    let out = std::path::PathBuf::from(&args[2]);
    std::fs::create_dir_all(&out).unwrap();
    let (mut ok, mut total) = (0, 0);
    let mut errors: BTreeMap<String, usize> = BTreeMap::new();
    for seed in 0..n {
        let (case, qs) = case_queries().generate(&mut Source::new(seed, (seed % 101) as usize));
        let m = final_model(&case);
        for (k, q) in qs.iter().enumerate() {
            let sql = q.sql();
            let file = out.join(format!("s{seed}_q{k}.sql"));
            std::fs::write(&file, ddl::pair_file(&m, &sql, &sql, "self")).unwrap();
            total += 1;
            match tools::lower(&file, &out.join(format!("s{seed}_q{k}.json"))) {
                Ok(()) => ok += 1,
                Err(e) => {
                    // The error's kind: digits and quoted names masked.
                    let kind: String = e.chars().take(90).map(|c| if c.is_ascii_digit() { '#' } else { c }).collect();
                    *errors.entry(kind).or_default() += 1;
                },
            }
        }
    }
    println!("{ok} of {total} lowered");
    let mut v: Vec<_> = errors.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    for (e, k) in v.iter().take(30) {
        println!("{k:5}  {e}");
    }
}
