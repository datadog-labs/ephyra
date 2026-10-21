// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Re-run `sqleq-solver` (resuming after HANGs) on a run's saved jobs and
//! rewrite the solver and finding columns of its `results.tsv`:
//! `cargo run --release --example resolve -- <run dir>`. For runs made
//! before `check::solve_all` resumed.

use ephyra_sqleq::check::solve_all;

fn main() {
    let run = std::path::PathBuf::from(std::env::args().nth(1).unwrap());
    let mut jobs = String::new();
    let mut names: Vec<_> = std::fs::read_dir(run.join("jobs")).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).collect();
    names.sort();
    for p in names {
        jobs += &std::fs::read_to_string(p).unwrap();
    }
    let dir = run.join("resolve");
    std::fs::create_dir_all(&dir).unwrap();
    let verdicts = solve_all(&jobs, &dir);
    let tsv = std::fs::read_to_string(run.join("results.tsv")).unwrap();
    let mut out = String::new();
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for (i, line) in tsv.lines().enumerate() {
        if i == 0 {
            out += line;
            out.push('\n');
            continue;
        }
        let mut f: Vec<String> = line.split('\t').map(String::from).collect();
        if let Some(v) = verdicts.get(&f[0]) {
            f[5] = v.clone();
        }
        // Re-derive the finding for an equivalent pair now proved or not.
        let (equivalent, lowered, qed) = (f[1] == "true", f[3] == "yes", f[4] == "true");
        let proved = qed || f[5] == "EQ";
        if !equivalent && f[2] == "true" && proved && !f[7].starts_with("known") {
            f[7] = if ephyra_sqleq::check::has_rounded_real(&std::fs::read_to_string(run.join("pairs").join(format!("{}.sql", f[0]))).unwrap_or_default()) { "known-qed-13-unsound".into() } else { "proves-refuted".into() };
        } else if f[7] == "unproved" && proved {
            f[7] = "-".into();
        } else if f[7] == "-" && equivalent && lowered && !proved && f[6] != "NOT-EQUIVALENT" {
            f[7] = "unproved".into();
        }
        *counts.entry(f[7].clone()).or_default() += 1;
        out += &f.join("\t");
        out.push('\n');
    }
    std::fs::write(run.join("results.tsv"), out).unwrap();
    println!("{} verdicts; findings now: {counts:?}", verdicts.len());
}
