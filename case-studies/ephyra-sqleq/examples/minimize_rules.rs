//! One minimal unproved pair per rule, from a one-step run's
//! `rule-blind-spots.txt` (see `selfcheck`): `cargo run --release --example
//! minimize_rules -- <run dir> [seconds per rule] [threads]`. For each rule
//! the pair with the shortest seed is minimized keeping that rule
//! (`minimize::minimize_rule`), written to `<run>/minimal-rules/<rule>.sql`.

use std::{collections::BTreeMap, sync::Mutex, time::Duration};

use ephyra_sqleq::{ddl, inputs, minimize};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let run = std::path::PathBuf::from(&args[1]);
    let secs: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(180);
    let threads: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(4);
    let out = run.join("minimal-rules-nopanic");
    std::fs::create_dir_all(&out).unwrap();
    // Per rule, the candidate with the shortest seed.
    let mut best: BTreeMap<String, (usize, u64, usize, usize)> = BTreeMap::new();
    let panicked: std::collections::BTreeSet<String> = std::fs::read_to_string(run.join("panics/list.txt")).unwrap_or_default().lines().map(String::from).collect();
    for n in std::fs::read_to_string(run.join("rule-blind-spots.txt")).unwrap().lines() {
        if panicked.contains(n) {
            continue;
        }
        let p: Vec<&str> = n.split('_').collect();
        let (s, k, j): (u64, usize, usize) = (p[0][1..].parse().unwrap(), p[1][1..].parse().unwrap(), p[p.len() - 1].parse().unwrap());
        let rule = p[2..p.len() - 1].join("_");
        let len = inputs::input(s).seeds[k].0.sql().len();
        if best.get(&rule).is_none_or(|b| len < b.0) {
            best.insert(rule, (len, s, k, j));
        }
    }
    let work: Mutex<Vec<(String, (usize, u64, usize, usize))>> = Mutex::new(best.into_iter().collect());
    std::thread::scope(|scope| {
        for t in 0..threads {
            let (work, out) = (&work, &out);
            scope.spawn(move || loop {
                let Some((rule, (_, s, k, j))) = work.lock().unwrap().pop() else { break };
                let input = inputs::input(s);
                let a = input.seeds[k].0.clone();
                // The j-th one-step variant of this seed in the run (at most two
                // per rule were kept, in the order one_step gives them).
                let mut per: BTreeMap<String, usize> = BTreeMap::new();
                let kept: Vec<_> = ephyra_sqleq::onestep::one_step(&a, &input.schema)
                    .into_iter()
                    .filter(|(r, _)| {
                        let n = per.entry(r.clone()).or_default();
                        *n += 1;
                        *n <= 2
                    })
                    .collect();
                let Some((r, b)) = kept.get(j).cloned() else { continue };
                assert_eq!(r, rule);
                let before = a.sql().len() + b.sql().len();
                let dir = std::env::temp_dir().join(format!("ephyra-sqleq-minr-{}-{t}", std::process::id()));
                // Unproved without a QED panic: a panic is a crash, found and
                // minimized apart (minimize_panics).
                let m = &input.model;
                let pdir = dir.join("panic");
                let holds = |x: &ephyra_postgresql::expr::Query, y: &ephyra_postgresql::expr::Query| minimize::unproved(m, x, y, &dir) && minimize::qed_panic_site(m, x, y, &pdir).is_none();
                if !holds(&a, &b) {
                    continue;
                }
                let (a2, b2) = minimize::minimize_rule_with(&input.schema, a, b, &rule, Duration::from_secs(secs), &holds);
                let after = a2.sql().len() + b2.sql().len();
                let m = minimize::narrowed(&input.model, &a2, &b2);
                std::fs::write(out.join(format!("{rule}.sql")), ddl::pair_file(&m, &a2.sql(), &b2.sql(), &format!("rule {rule}, minimized from s{s}_q{k} ({before} -> {after} chars)"))).unwrap();
                println!("{rule}: {before} -> {after}");
            });
        }
    });
}
