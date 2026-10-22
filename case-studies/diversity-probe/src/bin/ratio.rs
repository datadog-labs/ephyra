// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Where an input's effort goes, and what more queries per database would
//! change. Part 1, 5000 inputs as drawn: drawing the case (file and
//! statements) against drawing its queries, and the oracle's time, with
//! loading the database timed apart. Part 2, 300 databases: for k = 3, 30,
//! 300 queries drawn on the same database (fresh `Queries` draws on its final
//! schema), the time per query under each oracle and queries per second.
use std::time::Instant;
use ephyra::{Gen, Source};
use ephyra_sqlite::{cases::{case_metrics, Case, Cases}, expr::{Queries, Query}, probes::final_schema, sqlancer::{self, Tally}};
type Oracle = fn(&Case, &[Query]) -> Result<Tally, String>;
const ORACLES: [(&str, Oracle); 7] = [("norec", sqlancer::norec), ("tlp", sqlancer::tlp), ("plans", sqlancer::plans), ("order", sqlancer::order), ("setops", sqlancer::setops), ("cte", sqlancer::ctes), ("window", sqlancer::windows)];
fn mix(seed: u64, index: u64) -> u64 {
    let mut z = seed.wrapping_add(index.wrapping_mul(0x9e37_79b9_7f4a_7c15));
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}
fn main() {
    std::panic::set_hook(Box::new(|_| {}));
    let n = 5000u64;
    let (mut t_case, mut t_q, mut t_open, mut t_oracle) = (0f64, 0f64, 0f64, 0f64);
    let (mut tables, mut rows, mut stmts, mut queries, mut comps, mut no_q) = (0f64, 0f64, 0f64, 0f64, 0f64, 0u64);
    for i in 0..n {
        let mut g = Source::new(mix(5, i), (i % 101) as usize);
        let t = Instant::now();
        let case = Cases.generate(&mut g);
        t_case += t.elapsed().as_secs_f64();
        let t = Instant::now();
        let schema = final_schema(&case);
        let qs = if schema.tables.is_empty() { Vec::new() } else { Queries(schema).generate(&mut g) };
        t_q += t.elapsed().as_secs_f64();
        let m: std::collections::HashMap<_, _> = case_metrics(&case, 0).into_iter().collect();
        tables += m["tables"]; rows += m["rows"]; stmts += m["statements"];
        queries += qs.len() as f64;
        no_q += u64::from(qs.is_empty());
        let t = Instant::now();
        let _ = sqlancer::open(&case);
        t_open += t.elapsed().as_secs_f64();
        let t = Instant::now();
        if let Ok(tally) = ORACLES[(i % 7) as usize].1(&case, &qs) { comps += tally.comparisons as f64; }
        t_oracle += t.elapsed().as_secs_f64();
    }
    let f = n as f64;
    let total = t_case + t_q + t_oracle;
    println!("== Part 1: {n} inputs as drawn");
    println!("per input (ms): draw case {:.2}, draw queries {:.2}, oracle {:.2}; loading the database alone {:.2}", 1e3 * t_case / f, 1e3 * t_q / f, 1e3 * t_oracle / f, 1e3 * t_open / f);
    println!("share of time: drawing the case {:.0}%, drawing queries {:.0}%, oracle {:.0}%; loading the database is about {:.0}% of the oracle's time", 100.0 * t_case / total, 100.0 * t_q / total, 100.0 * t_oracle / total, 100.0 * t_open / t_oracle);
    println!("per input: {:.2} tables, {:.0} rows, {:.1} statements; {:.2} queries ({:.1}% have none); {:.2} comparisons ({:.2} per query)", tables / f, rows / f, stmts / f, queries / f, 100.0 * no_q as f64 / f, comps / f, comps / queries);

    println!("== Part 2: 300 databases, k queries each, every oracle on the same k");
    for k in [3usize, 30, 300] {
        let (mut t_db, mut t_draw, mut nq) = (0f64, 0f64, 0f64);
        let mut per_oracle = [0f64; 7];
        let mut comps = [0f64; 7];
        for i in 0..300u64 {
            let mut g = Source::new(mix(11, i), (i % 101) as usize);
            let t = Instant::now();
            let case = Cases.generate(&mut g);
            t_db += t.elapsed().as_secs_f64();
            let schema = final_schema(&case);
            if schema.tables.is_empty() { continue; }
            let t = Instant::now();
            let mut qs = Vec::new();
            while qs.len() < k { qs.extend(Queries(schema.clone()).generate(&mut g)); }
            qs.truncate(k);
            t_draw += t.elapsed().as_secs_f64();
            nq += k as f64;
            for (o, (_, oracle)) in ORACLES.iter().enumerate() {
                let t = Instant::now();
                if let Ok(tally) = oracle(&case, &qs) { comps[o] += tally.comparisons as f64; }
                per_oracle[o] += t.elapsed().as_secs_f64();
            }
        }
        let run: f64 = per_oracle.iter().sum::<f64>() / 7.0; // one oracle per database, on average
        let per_q: Vec<String> = ORACLES.iter().zip(per_oracle.iter()).map(|((nm, _), t)| format!("{nm} {:.2}", 1e3 * t / nq)).collect();
        println!("k={k:3}: ms per query by oracle: {}", per_q.join(", "));
        println!("       one oracle per database: {:.0} queries/s, {:.0} comparisons/s (case draw {:.0}%, query draw {:.0}%, oracle {:.0}% of the time)",
            nq / (t_db + t_draw + run), comps.iter().sum::<f64>() / 7.0 / (t_db + t_draw + run),
            100.0 * t_db / (t_db + t_draw + run), 100.0 * t_draw / (t_db + t_draw + run), 100.0 * run / (t_db + t_draw + run));
        let all: f64 = per_oracle.iter().sum();
        println!("       all 7 oracles per database: {:.0} query checks/s, {:.0} comparisons/s", 7.0 * nq / (t_db + t_draw + all), comps.iter().sum::<f64>() / (t_db + t_draw + all));
    }
}
