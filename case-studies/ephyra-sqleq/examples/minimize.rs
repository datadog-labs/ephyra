// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Minimize the unproved pairs of a run: `cargo run --release --example
//! minimize -- <run dir> [seconds per pair] [max pairs]`. Reads
//! `<run>/results.tsv`, regenerates each unproved pair from its name
//! (`s{seed}_q{k}_eq{j}`), and writes `<run>/minimal/<name>.sql`, plus
//! `<run>/minimal/index.tsv` (name, size before and after).

use std::time::Duration;

use ephyra_sqleq::{ddl, inputs, minimize};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let run = std::path::PathBuf::from(&args[1]);
    let secs: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(120);
    let max: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(usize::MAX);
    let out = run.join("minimal");
    std::fs::create_dir_all(&out).unwrap();
    let tsv = std::fs::read_to_string(run.join("results.tsv")).unwrap();
    let names: Vec<String> = tsv.lines().skip(1).filter(|l| l.ends_with("\tunproved")).map(|l| l.split('\t').next().unwrap().to_string()).take(max).collect();
    let mut index = String::from("pair\tbefore\tafter\n");
    for name in names {
        let nums: Vec<u64> = name.split(|c: char| !c.is_ascii_digit()).filter(|x| !x.is_empty()).map(|x| x.parse().unwrap()).collect();
        let (s, k, j) = (nums[0], nums[1] as usize, nums[2] as usize);
        let input = inputs::input(s);
        let (a, vs) = &input.seeds[k];
        let b = vs[j].clone();
        let before = a.sql().len() + b.sql().len();
        let work = std::env::temp_dir().join(format!("ephyra-sqleq-min-{}", std::process::id()));
        let (a2, b2) = minimize::minimize(&input.model, &input.schema, a.clone(), b, &work, Duration::from_secs(secs));
        let after = a2.sql().len() + b2.sql().len();
        let m = minimize::narrowed(&input.model, &a2, &b2);
        std::fs::write(out.join(format!("{name}.sql")), ddl::pair_file(&m, &a2.sql(), &b2.sql(), &format!("minimized from {name}"))).unwrap();
        index += &format!("{name}\t{before}\t{after}\n");
        println!("{name}: {before} -> {after} chars");
    }
    std::fs::write(out.join("index.tsv"), index).unwrap();
}
