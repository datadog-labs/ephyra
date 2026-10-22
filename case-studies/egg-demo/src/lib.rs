// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Equivalent SQL queries sampled from an e-graph, and the tools around
//! them: the sampler, sqleq's checks, and generated seeds.

pub mod check;
pub mod lang;
pub mod parse;
pub mod print;
pub mod sample;
pub mod seed;
pub mod sqlite;

use std::time::Duration;

use egg::{BackoffScheduler, RecExpr, Runner};

use crate::{
    lang::{Facts, Sql},
    sample::Rng,
};

pub fn read(path: &str) -> Result<parse::Input, String> {
    let script = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    parse::read(&script)
}

/// An sqleq input file: the DDL, the seed query as written, and `other`.
pub fn pair_file(input: &parse::Input, path: &str, other: &str) -> String {
    let ddl: Vec<String> = input.ddl.iter().map(|d| format!("{d};")).collect();
    format!("-- A pair from egg-demo, seed {path}.\n{}\n{};\n{other};\n", ddl.join("\n"), input.query)
}

/// The seed as printed, and the `n` most structurally different of
/// `samples` variants drawn from its e-graph (SQL and term). `report` prints
/// what the e-graph and the sampler did.
pub fn variants(
    input: &parse::Input,
    n: usize,
    samples: u64,
    seed: u64,
    report: bool,
) -> (String, Vec<(String, RecExpr<Sql>)>) {
    let seed_sql = print::sql(&input.term).expect("the seed prints");
    let runner: Runner<Sql, Facts> = Runner::new(Facts { not_null: input.not_null.clone(), keys: input.keys.clone() })
        .with_expr(&input.term)
        .with_node_limit(20_000)
        .with_iter_limit(12)
        .with_time_limit(Duration::from_secs(5))
        .with_scheduler(BackoffScheduler::default())
        .run(&lang::rules());
    let (eg, root) = (&runner.egraph, runner.roots[0]);
    let size = input.term.as_ref().len();
    let in_seed = |s: &str| input.term.as_ref().iter().any(|n| n.to_string() == s);
    let banned = ["TRUE", "FALSE"].into_iter().filter(|s| !in_seed(s)).map(egg::Symbol::from).collect();
    let counts = sample::count(eg, size + size / 2 + 6, banned);

    let mut rng = Rng::new(seed);
    let (mut drawn, mut refused) = (Vec::<(String, RecExpr<Sql>)>::new(), 0);
    for _ in 0..samples {
        let Some(term) = sample::draw(eg, &counts, root, &mut rng) else { continue };
        match print::sql(&term) {
            Some(sql) if sql != seed_sql && !drawn.iter().any(|(s, _)| *s == sql) => drawn.push((sql, term)),
            Some(_) => {},
            None => refused += 1,
        }
    }
    let fingerprints: Vec<_> = drawn.iter().map(|(_, t)| sample::fingerprint(t)).collect();
    let chosen = sample::spread(&sample::fingerprint(&input.term), &fingerprints, n);

    if report {
        let mut fired: std::collections::BTreeMap<String, usize> = Default::default();
        for iteration in &runner.iterations {
            for (rule, n) in &iteration.applied {
                *fired.entry(rule.to_string()).or_default() += n;
            }
        }
        println!(
            "e-graph: {} e-classes, {} e-nodes after {} iterations (stopped: {:?})",
            eg.number_of_classes(),
            eg.total_number_of_nodes(),
            runner.iterations.len(),
            runner.stop_reason.as_ref().unwrap()
        );
        let fired: Vec<String> = fired.iter().map(|(r, n)| format!("{r} {n}")).collect();
        println!("rules that fired (times): {}", fired.join(", "));
        let total: f64 = counts.of(eg.find(root)).iter().sum();
        println!("equivalent terms of up to {} nodes in the root class: {total:.3e}", counts.max);
        println!(
            "sampled {samples}: {} distinct and printable, {refused} with no plain-SQL rendering; kept {} by structural spread",
            drawn.len(),
            chosen.len()
        );
        let kept: Vec<_> = chosen.iter().map(|&i| &fingerprints[i]).collect();
        println!(
            "mean structural distance: {:.2} among those kept, {:.2} among all drawn\n",
            sample::mean_distance(&kept),
            sample::mean_distance(&fingerprints.iter().collect::<Vec<_>>())
        );
    }
    let chosen = chosen.into_iter().map(|i| drawn[i].clone()).collect();
    (seed_sql, chosen)
}
