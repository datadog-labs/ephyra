//! Coverage found per second for k queries per database: for each setting,
//! 120 s of databases (fresh Cases draws) with k queries each (fresh Queries
//! draws on the final schema), each database under one oracle in turn;
//! distinct SQLite counters reached at 30, 60 and 120 s, and comparisons.
//! `mixed` draws k log-uniformly from 1 to 300 per database.
use std::{collections::HashSet, time::{Duration, Instant}};
use ephyra::{coverage, Gen, Source};
use ephyra_sqlite::{cases::{Case, Cases}, expr::{Queries, Query}, probes::final_schema, sqlancer::{self, Tally}};
type Oracle = fn(&Case, &[Query]) -> Result<Tally, String>;
const ORACLES: [Oracle; 7] = [sqlancer::norec, sqlancer::tlp, sqlancer::plans, sqlancer::order, sqlancer::setops, sqlancer::ctes, sqlancer::windows];
fn mix(seed: u64, index: u64) -> u64 {
    let mut z = seed.wrapping_add(index.wrapping_mul(0x9e37_79b9_7f4a_7c15));
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}
fn main() {
    coverage::suppress_default_dump();
    std::panic::set_hook(Box::new(|_| {}));
    for i in 0..100u64 {
        let mut g = Source::new(mix(7, i), i as usize);
        let c = Cases.generate(&mut g);
        let _ = sqlancer::open(&c);
    }
    for (name, k) in [("k=3", Some(3usize)), ("k=30", Some(30)), ("k=300", Some(300)), ("mixed", None)] {
        let mut seen: HashSet<usize> = HashSet::new();
        let (mut dbs, mut queries, mut comps) = (0u64, 0u64, 0u64);
        let start = Instant::now();
        let mut marks = vec![];
        let mut i = 0u64;
        while start.elapsed() < Duration::from_secs(120) {
            i += 1;
            let mut g = Source::new(mix(0xd15c0 + name.len() as u64, i), (i % 101) as usize);
            let case = Cases.generate(&mut g);
            let schema = final_schema(&case);
            if schema.tables.is_empty() { continue; }
            let k = k.unwrap_or_else(|| (300f64.powf((mix(3, i) % 10_000) as f64 / 10_000.0)).round().max(1.0) as usize);
            let mut qs = Vec::new();
            while qs.len() < k { qs.extend(Queries(schema.clone()).generate(&mut g)); }
            qs.truncate(k);
            sqlancer::set_deadline(Some(Instant::now() + Duration::from_secs(30)));
            let before = coverage::snapshot().unwrap();
            if let Ok(t) = ORACLES[(i % 7) as usize](&case, &qs) { comps += t.comparisons; }
            let delta = before.delta(&coverage::snapshot().unwrap());
            sqlancer::set_deadline(None);
            seen.extend(delta.hits().iter().map(|&(c, _)| c));
            dbs += 1;
            queries += k as u64;
            let s = start.elapsed().as_secs();
            if (s >= 30 && marks.is_empty()) || (s >= 60 && marks.len() == 1) { marks.push(seen.len()); }
        }
        marks.push(seen.len());
        println!("{name:6} counters at 30/60/120 s: {:?}; {dbs} databases, {queries} queries, {comps} comparisons in 120 s", marks);
    }
}
