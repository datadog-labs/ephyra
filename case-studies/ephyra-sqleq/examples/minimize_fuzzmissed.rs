//! Minimal reproductions of near-miss pairs sqleq-fuzz misses: the seed
//! shrunk keeping an anti-rule variant that PostgreSQL refutes (a witness
//! exists) and sqleq-fuzz does not, then the witness data shrunk to the
//! fewest rows. `cargo run --release --example minimize_fuzzmissed -- <run
//! dir> [seconds per pair]`. Writes `<run>/minimal-fuzzmissed/<pair>.sql`:
//! DDL, the two queries, the witness rows, and both results on PostgreSQL.

use std::time::Duration;

use ephyra_postgresql::expr::Query;
use ephyra_sqleq::{antirules, ddl, inputs, minimize, tools, variants, witness};

fn fuzz_misses(m: &ephyra_postgresql::model::Model, a: &Query, b: &Query, dir: &std::path::Path) -> bool {
    let f = dir.join("p.sql");
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(&f, ddl::pair_file(m, &a.sql(), &b.sql(), "fuzz")).unwrap();
    let o = std::process::Command::new(tools::bin("sqleq-fuzz")).arg("file").arg(&f).output().unwrap();
    String::from_utf8_lossy(&o.stdout).starts_with("NO-COUNTEREXAMPLE")
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let run = std::path::PathBuf::from(&args[1]);
    let secs: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(180);
    let out = run.join("minimal-fuzzmissed");
    std::fs::create_dir_all(&out).unwrap();
    let tsv = std::fs::read_to_string(run.join("results.tsv")).unwrap();
    let names: Vec<String> = tsv.lines().skip(1).filter(|l| l.split('\t').nth(7) == Some("fuzz-missed")).map(|l| l.split('\t').next().unwrap().to_string()).collect();
    for name in names {
        let p: Vec<&str> = name.split('_').collect();
        let (s, k, j): (u64, usize, usize) = (p[0][1..].parse().unwrap(), p[1][1..].parse().unwrap(), p[p.len() - 1].parse().unwrap());
        let disguised = p[p.len() - 2].ends_with("-d");
        let rule = p[2..p.len() - 1].join("_").trim_end_matches("-d").to_string();
        let input = inputs::input(s);
        let seed = input.seeds[k].0.clone();
        let (_, v) = antirules::anti(&seed, &input.schema).into_iter().nth(j).unwrap();
        let v = if disguised { variants::variants(&v, &input.schema, 1, 30, s ^ j as u64).0.into_iter().next().unwrap_or(v) } else { v };
        let dir = std::env::temp_dir().join(format!("ephyra-sqleq-fm-{}", std::process::id()));
        let (case, m) = (&input.case, &input.model);
        // Shrink keeping a plain (undisguised) variant by the same rule.
        let holds = |a: &Query, b: &Query| witness::witness(case, m, a, b).ok().flatten().is_some() && fuzz_misses(m, a, b, &dir);
        if !holds(&seed, &v) {
            println!("{name}: no longer reproduces");
            continue;
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(secs);
        let (mut a, mut b) = (seed, v);
        'outer: while std::time::Instant::now() < deadline {
            for c in ephyra_postgresql::expr::simpler_queries(&a) {
                if std::time::Instant::now() > deadline {
                    break 'outer;
                }
                if !c.valid(&input.schema) || c.cost() >= a.cost() {
                    continue;
                }
                if let Some((_, v2)) = antirules::anti(&c, &input.schema).into_iter().find(|(r, v2)| *r == rule && holds(&c, v2)) {
                    a = c;
                    b = v2;
                    continue 'outer;
                }
            }
            break;
        }
        let w = witness::witness(case, m, &a, &b).ok().flatten().unwrap_or_default();
        let rows = witness::minimal_rows(case, m, &a, &b, &w).unwrap_or_else(|e| vec![format!("-- {e}")]);
        let mm = minimize::narrowed(m, &a, &b);
        // Both results on the witness rows.
        let results = ephyra_postgresql::pg::with_session(|sess| {
            sess.fresh()?;
            sess.execute(&ddl::schema(&mm))?;
            for r in &rows {
                if !r.starts_with("--") {
                    sess.execute(r)?;
                }
            }
            let ra = sess.query(&a.sql())?;
            let rb = sess.query(&b.sql())?;
            Ok(format!("-- on the witness rows, query 1 returns {} rows, query 2 returns {} rows:\n-- 1: {:?}\n-- 2: {:?}", ra.len(), rb.len(), ra, rb))
        })
        .unwrap_or_else(|e| format!("-- {e}"));
        let text = format!(
            "-- {rule}: a pair sqleq-fuzz finds no counterexample for; PostgreSQL shows they differ on the rows below.\n-- minimized from {name}\n{}{};\n{};\n\n-- witness rows (load into the schema above):\n{}\n\n{results}\n",
            ddl::schema(&mm),
            a.sql(),
            b.sql(),
            rows.iter().map(|r| format!("{r};")).collect::<Vec<_>>().join("\n")
        );
        std::fs::write(out.join(format!("{name}.sql")), text).unwrap();
        println!("{name}: {} witness rows, {} chars of SQL", rows.len(), a.sql().len() + b.sql().len());
    }
}
