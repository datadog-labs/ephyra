// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! One minimal pair per QED panic site, from a one-step run whose panics
//! were listed (`<run>/panics/sites.txt`, lines `site ..|pair`):
//! `cargo run --release --example minimize_panics -- <run dir> [seconds] [threads]`.
//! Writes `<run>/minimal-panics/<file>_<line>.sql`.

use std::{collections::BTreeMap, sync::Mutex, time::Duration};

use ephyra_sqleq::{ddl, inputs, minimize};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let run = std::path::PathBuf::from(&args[1]);
    let secs: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(240);
    let threads: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(5);
    let out = run.join("minimal-panics");
    std::fs::create_dir_all(&out).unwrap();
    // Per site, the pair with the shortest seed.
    let mut best: BTreeMap<String, (usize, String)> = BTreeMap::new();
    for l in std::fs::read_to_string(run.join("panics/sites.txt")).unwrap().lines() {
        let (site, name) = l.split_once('|').unwrap();
        // One-step equivalent pairs only (a mutant is not one rule away).
        if name.contains("_mut") {
            continue;
        }
        let Some(site) = site.split(' ').next().and_then(|s| s.rsplitn(2, ':').nth(1)) else { continue };
        let len = std::fs::read_to_string(run.join("pairs").join(format!("{name}.sql"))).map_or(usize::MAX, |t| t.len());
        if ["normal.rs:655", "partial.rs:185"].iter().any(|d| site.contains(d)) && std::env::var("ALL").is_err() {
            continue;
        }
        if best.get(site).is_none_or(|b| len < b.0) {
            best.insert(site.to_string(), (len, name.to_string()));
        }
    }
    let work: Mutex<Vec<(String, String)>> = Mutex::new(best.into_iter().map(|(s, (_, n))| (s, n)).collect());
    std::thread::scope(|scope| {
        for t in 0..threads {
            let (work, out) = (&work, &out);
            scope.spawn(move || loop {
                let Some((site, name)) = work.lock().unwrap().pop() else { break };
                let p: Vec<&str> = name.split('_').collect();
                let (s, k, j): (u64, usize, usize) = (p[0][1..].parse().unwrap(), p[1][1..].parse().unwrap(), p[p.len() - 1].parse().unwrap());
                let rule = p[2..p.len() - 1].join("_");
                let input = inputs::input(s);
                let a = input.seeds[k].0.clone();
                let mut per: BTreeMap<String, usize> = BTreeMap::new();
                let kept: Vec<_> = ephyra_sqleq::onestep::one_step(&a, &input.schema)
                    .into_iter()
                    .filter(|(r, _)| {
                        let n = per.entry(r.clone()).or_default();
                        *n += 1;
                        *n <= 2
                    })
                    .collect();
                let Some((_, b)) = kept.get(j).cloned() else { continue };
                let dir = std::env::temp_dir().join(format!("ephyra-sqleq-panic-{}-{t}", std::process::id()));
                let m = &input.model;
                let holds = |x: &ephyra_postgresql::expr::Query, y: &ephyra_postgresql::expr::Query| minimize::qed_panic_site(m, x, y, &dir).as_deref() == Some(site.as_str());
                if !holds(&a, &b) {
                    println!("{site}: {name} does not panic there any more");
                    continue;
                }
                let before = a.sql().len() + b.sql().len();
                let (a2, b2) = minimize::minimize_rule_with(&input.schema, a, b, &rule, Duration::from_secs(secs), &holds);
                let after = a2.sql().len() + b2.sql().len();
                let mm = minimize::narrowed(m, &a2, &b2);
                let file = site.replace(['/', '.'], "_").replace(':', "_");
                std::fs::write(out.join(format!("{file}.sql")), ddl::pair_file(&mm, &a2.sql(), &b2.sql(), &format!("QED panics at {site}; rule {rule}, minimized from {name} ({before} -> {after} chars)"))).unwrap();
                println!("{site}: {before} -> {after}");
            });
        }
    });
}
