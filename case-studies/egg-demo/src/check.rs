//! `check`: every sampled pair, and a one-operator mutant of each, past
//! sqleq's frontend, its two provers (QED, `sqleq-solver`), its disprover
//! (`sqleq-fuzz`) and this demo's own SQLite check, as one cross-tab.
//!
//! Two cells are alarms. A prover that proves a pair the fuzzer or SQLite
//! refutes is unsound, or was fed unfaithful IR. A pair this demo calls
//! equivalent that either refutes means one of its rules, or its printer, is
//! wrong. The run exits non-zero on either.
//!
//! Tools are found from a flag, then an environment variable, never a
//! default: `--sqleq-bin DIR` or `$SQLEQ_BIN` (a directory holding
//! `sqleq-frontend`, `sqleq-solver` and `sqleq-fuzz`, such as sqleq's
//! `target/release`), and `--qed PATH` or `$QED_PROVER`. A tool not given is
//! reported as not run.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use egg::{Id, Language, RecExpr};

use crate::{lang::Sql, pair_file, print, read, sample::Rng, sqlite, variants};

/// One pair and what each check said of it; `None` where it did not run.
#[derive(Default)]
struct Row {
    name: String,
    equivalent: bool,
    file: PathBuf,
    /// Refuted on some random SQLite database; `Err` when the query does not run.
    sqlite: Option<Result<bool, String>>,
    fuzz: Option<String>,
    lowered: Option<bool>,
    qed: Option<bool>,
    solver: Option<String>,
}

impl Row {
    fn refuted(&self) -> bool {
        matches!(self.sqlite, Some(Ok(true))) || self.fuzz.as_deref() == Some("NOT-EQUIVALENT")
    }

    fn proved(&self) -> bool {
        self.qed == Some(true) || self.solver.as_deref() == Some("EQ")
    }
}

pub fn run(args: &[String], n: usize, samples: u64, seed: u64) -> i32 {
    let value = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
    let with_value = ["--out", "--n", "--samples", "--seed", "--sqleq-bin", "--qed"];
    let mut seeds = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if with_value.contains(&args[i].as_str()) {
            i += 2;
        } else {
            seeds.push(args[i].clone());
            i += 1;
        }
    }
    let Some(out) = value("--out").map(PathBuf::from) else {
        eprintln!("check needs --out DIR for the pairs, the IR and the verdicts");
        return 2;
    };
    let sqleq = value("--sqleq-bin").or_else(|| std::env::var("SQLEQ_BIN").ok()).map(PathBuf::from);
    let qed = value("--qed").or_else(|| std::env::var("QED_PROVER").ok()).map(PathBuf::from);
    let tool = |name: &str| sqleq.as_ref().map(|d| d.join(name));
    for path in [tool("sqleq-frontend"), tool("sqleq-solver"), tool("sqleq-fuzz"), qed.clone()].iter().flatten() {
        if !path.is_file() {
            eprintln!("{} does not exist", path.display());
            return 2;
        }
    }

    let _ = fs::remove_dir_all(&out);
    for dir in ["pairs", "ir", "qed"] {
        fs::create_dir_all(out.join(dir)).expect("the output directories are created");
    }
    let mut rows: Vec<Row> = Vec::new();
    let mut rng = Rng::new(seed ^ 0x6d75_7461);
    for path in &seeds {
        let input = match read(path) {
            Ok(input) => input,
            Err(e) => {
                println!("{path}: refused: {e}");
                continue;
            },
        };
        let stem = Path::new(path).file_stem().unwrap().to_string_lossy().to_string();
        let (seed_sql, chosen) = variants(&input, n, samples, seed, false);
        let mut pairs: Vec<(String, bool, String)> = Vec::new();
        for (k, (sql, term)) in chosen.iter().enumerate() {
            pairs.push((format!("{stem}_eq_{:02}", k + 1), true, sql.clone()));
            let mutant = mutate(term, &mut rng).and_then(|m| print::sql(&m));
            if let Some(m) = mutant.filter(|m| *m != *sql && *m != seed_sql) {
                pairs.push((format!("{stem}_mut_{:02}", k + 1), false, m));
            }
        }
        let others: Vec<&String> = pairs.iter().map(|p| &p.2).collect();
        let outcomes = sqlite::check(&input, &seed_sql, &others, 300, seed);
        for ((name, equivalent, sql), (differs, error)) in pairs.into_iter().zip(outcomes) {
            let file = out.join("pairs").join(format!("{name}.sql"));
            fs::write(&file, pair_file(&input, path, &sql)).expect("the pair is written");
            let sqlite = Some(error.map_or(Ok(differs > 0), Err));
            rows.push(Row { name, equivalent, file, sqlite, ..Row::default() });
        }
    }

    if let Some(fuzz) = tool("sqleq-fuzz") {
        for row in &mut rows {
            let out = Command::new(&fuzz).arg("file").arg(&row.file).output().expect("sqleq-fuzz runs");
            let text = String::from_utf8_lossy(&out.stdout);
            row.fuzz = Some(text.lines().next().unwrap_or("").split_whitespace().next().unwrap_or("").to_string());
        }
    }
    if let Some(frontend) = tool("sqleq-frontend") {
        let jobs = out.join("jobs.jsonl");
        let mut all_jobs = String::new();
        for row in &mut rows {
            let ir = out.join("ir").join(format!("{}.json", row.name));
            let lowered = Command::new(&frontend).arg(&row.file).arg(&ir).output().expect("the frontend runs");
            row.lowered = Some(lowered.status.success());
            if !lowered.status.success() {
                continue;
            }
            fs::copy(&ir, out.join("qed").join(format!("{}.json", row.name))).expect("the IR is copied");
            let job = out.join("ir").join(format!("{}.job", row.name));
            let packaged = Command::new(&frontend)
                .args(["--sqlsolver", "--ir"])
                .arg(&ir)
                .args(["--name", &row.name, "-o"])
                .arg(&job)
                .output()
                .expect("the frontend runs");
            if packaged.status.success() {
                all_jobs += &fs::read_to_string(&job).unwrap_or_default();
            }
        }
        fs::write(&jobs, all_jobs).expect("the jobs are written");
        if let Some(solver) = tool("sqleq-solver") {
            let results = out.join("solver.jsonl");
            Command::new(&solver)
                .arg(&jobs)
                .arg(&results)
                .arg("--timeout-ms=20000")
                .output()
                .expect("sqleq-solver runs");
            let verdicts: BTreeMap<String, String> = (fs::read_to_string(&results).unwrap_or_default().lines())
                .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
                .map(|v| (v["name"].as_str().unwrap_or("").to_string(), v["verdict"].as_str().unwrap_or("").to_string()))
                .collect();
            for row in rows.iter_mut().filter(|r| r.lowered == Some(true)) {
                row.solver = verdicts.get(&row.name).cloned();
            }
        }
        if let Some(qed) = &qed {
            Command::new(qed).arg(out.join("qed")).output().expect("qed-prover runs");
            for row in rows.iter_mut().filter(|r| r.lowered == Some(true)) {
                let result = out.join("qed").join(format!("{}.result", row.name));
                row.qed = fs::read_to_string(result)
                    .ok()
                    .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                    .and_then(|v| v["provable"].as_bool());
            }
        }
    }

    report(&rows, sqleq.is_some(), qed.is_some());
    write_tsv(&out, &rows);
    let prover_alarm = rows.iter().any(|r| r.proved() && r.refuted());
    let generator_alarm = rows.iter().any(|r| r.equivalent && (r.refuted() || matches!(r.sqlite, Some(Err(_)))));
    i32::from(prover_alarm || generator_alarm)
}

/// One node of `term`, chosen at random, changed so the query most likely
/// means something else: a comparison or connective swapped, a LEFT JOIN
/// made inner, DISTINCT dropped, EXISTS or IN negated.
fn mutate(term: &RecExpr<Sql>, rng: &mut Rng) -> Option<RecExpr<Sql>> {
    let nodes = term.as_ref();
    let candidates: Vec<usize> = (0..nodes.len())
        .filter(|&i| {
            matches!(
                nodes[i],
                Sql::Eq(_)
                    | Sql::Ne(_)
                    | Sql::Lt(_)
                    | Sql::Le(_)
                    | Sql::Gt(_)
                    | Sql::Ge(_)
                    | Sql::And(_)
                    | Sql::Or(_)
                    | Sql::LeftJoin(_)
                    | Sql::Distinct(_)
                    | Sql::GroupBy(_)
                    | Sql::Exists(_)
                    | Sql::InSub(_)
            )
        })
        .collect();
    let target = *candidates.get((rng.next_u64() % candidates.len().max(1) as u64) as usize)?;
    let mut out = RecExpr::default();
    copy(term, Id::from(nodes.len() - 1), target, &mut out);
    Some(out)
}

/// `term` from `id` down into `out`, with the node at `target` mutated.
fn copy(term: &RecExpr<Sql>, id: Id, target: usize, out: &mut RecExpr<Sql>) -> Id {
    let node = term[id].clone().map_children(|c| copy(term, c, target, out));
    if usize::from(id) != target {
        return out.add(node);
    }
    match node {
        Sql::Eq(c) => out.add(Sql::Ne(c)),
        Sql::Ne(c) => out.add(Sql::Eq(c)),
        Sql::Lt(c) => out.add(Sql::Le(c)),
        Sql::Le(c) => out.add(Sql::Lt(c)),
        Sql::Gt(c) => out.add(Sql::Ge(c)),
        Sql::Ge(c) => out.add(Sql::Gt(c)),
        Sql::And(c) => out.add(Sql::Or(c)),
        Sql::Or(c) => out.add(Sql::And(c)),
        Sql::LeftJoin([q, a, b]) => {
            let joined = out.add(Sql::Join([a, b]));
            out.add(Sql::Filter([q, joined]))
        },
        Sql::Distinct([p]) => p,
        Sql::GroupBy(c) => out.add(Sql::Project(c)),
        e @ (Sql::Exists(_) | Sql::InSub(_)) => {
            let e = out.add(e);
            out.add(Sql::Not([e]))
        },
        other => out.add(other),
    }
}

fn report(rows: &[Row], sqleq: bool, qed: bool) {
    let (eq, mutants): (Vec<&Row>, Vec<&Row>) = rows.iter().partition(|r| r.equivalent);
    let count = |rs: &[&Row], f: &dyn Fn(&Row) -> bool| rs.iter().filter(|r| f(r)).count();
    let line = |label: &str, ran: bool, f: &dyn Fn(&Row) -> bool| {
        if ran {
            println!("  {label:28} {:>5} / {:<5} {:>5} / {:<5}", count(&eq, f), eq.len(), count(&mutants, f), mutants.len());
        } else {
            println!("  {label:28} not run");
        }
    };
    println!("{} pairs: {} equivalent by construction, {} mutants", rows.len(), eq.len(), mutants.len());
    println!("  {:28} {:>13} {:>13}", "", "equivalent", "mutants");
    line("SQLite refutes", true, &|r| matches!(r.sqlite, Some(Ok(true))));
    line("SQLite cannot run it", true, &|r| matches!(r.sqlite, Some(Err(_))));
    line("sqleq-fuzz refutes", sqleq, &|r| r.fuzz.as_deref() == Some("NOT-EQUIVALENT"));
    line("frontend lowers", sqleq, &|r| r.lowered == Some(true));
    line("QED proves", sqleq && qed, &|r| r.qed == Some(true));
    line("sqleq-solver proves", sqleq, &|r| r.solver.as_deref() == Some("EQ"));
    line("sqleq-solver cannot translate", sqleq, &|r| r.solver.as_deref() == Some("NOTRANS"));

    let prover: Vec<&str> = rows.iter().filter(|r| r.proved() && r.refuted()).map(|r| r.name.as_str()).collect();
    let generator: Vec<&str> = (rows.iter())
        .filter(|r| r.equivalent && (r.refuted() || matches!(r.sqlite, Some(Err(_)))))
        .map(|r| r.name.as_str())
        .collect();
    let list = |names: &[&str]| if names.is_empty() { "none".to_string() } else { names.join(", ") };
    println!("alarm, a prover proves a refuted pair: {}", list(&prover));
    println!("alarm, a pair called equivalent is refuted or does not run: {}", list(&generator));
}

/// Every verdict per pair, for whoever wants to look at one.
fn write_tsv(out: &Path, rows: &[Row]) {
    let mut tsv = String::from("pair\tequivalent\tsqlite\tfuzz\tlowered\tqed\tsqleq-solver\n");
    let opt = |o: Option<String>| o.unwrap_or_else(|| "-".to_string());
    for r in rows {
        let sqlite = r.sqlite.as_ref().map(|s| match s {
            Ok(true) => "refutes".to_string(),
            Ok(false) => "agrees".to_string(),
            Err(e) => format!("error: {e}"),
        });
        tsv += &format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            r.name,
            r.equivalent,
            opt(sqlite),
            opt(r.fuzz.clone()),
            opt(r.lowered.map(|b| b.to_string())),
            opt(r.qed.map(|b| b.to_string())),
            opt(r.solver.clone())
        );
    }
    fs::write(out.join("results.tsv"), tsv).expect("the results are written");
}
