// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Memory probe: inputs drawn as the nightly runner draws them
//! (`--queries LO-HI` extension, `--mutate K` mutations, as examples/sqlancer.rs),
//! each run under an oracle with the runner's deadline (10 s + 0.1 s a
//! query); per input the peak of SQLite's allocations
//! (sqlite3_memory_highwater, reset per input) and of Rust's (a counting
//! allocator), printed when over `--report MB` (default 256).
//!
//! `cargo run --release --example heavy -- <oracle> <first seed> <count> [--queries LO-HI] [--mutate K] [--report MB] [--per-query]`;
//! with `--per-query` an input over the report is run again query by query
//! and its two heaviest printed. A worker the runner kills over its RSS
//! limit leaves no input: this finds the inputs that would have been.

use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicUsize, Ordering::Relaxed},
    time::{Duration, Instant},
};

use ephyra::{Gen, Source};
use ephyra_sqlite::{
    cases::Case,
    expr::Query,
    probes::{case_queries, extend, mix},
    sqlancer::{self, Tally},
};

struct Counting;
static CUR: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = System.alloc(l);
        if !p.is_null() {
            let c = CUR.fetch_add(l.size(), Relaxed) + l.size();
            PEAK.fetch_max(c, Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        CUR.fetch_sub(l.size(), Relaxed);
        System.dealloc(p, l)
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        let q = System.realloc(p, l, n);
        if !q.is_null() {
            if n > l.size() {
                let c = CUR.fetch_add(n - l.size(), Relaxed) + n - l.size();
                PEAK.fetch_max(c, Relaxed);
            } else {
                CUR.fetch_sub(l.size() - n, Relaxed);
            }
        }
        q
    }
}
#[global_allocator]
static A: Counting = Counting;

type Oracle = fn(&Case, &[Query]) -> Result<Tally, String>;
const MB: f64 = 1024.0 * 1024.0;

fn run(oracle: Oracle, c: &Case, qs: &[Query], secs: f64) -> (f64, f64, f64, bool, Option<String>) {
    unsafe { rusqlite::ffi::sqlite3_memory_highwater(1) };
    PEAK.store(CUR.load(Relaxed), Relaxed);
    let base = CUR.load(Relaxed);
    sqlancer::set_deadline(Some(Instant::now() + Duration::from_secs_f64(secs)));
    let t = Instant::now();
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| oracle(c, qs)));
    let ms = t.elapsed().as_secs_f64() * 1e3;
    let late = sqlancer::past_deadline();
    sqlancer::set_deadline(None);
    let sq = unsafe { rusqlite::ffi::sqlite3_memory_highwater(1) } as f64 / MB;
    let rs = (PEAK.load(Relaxed) - base) as f64 / MB;
    let e = match r {
        Ok(Err(e)) => Some(e),
        Err(_) => Some("panic".into()),
        _ => None,
    };
    (ms, sq, rs, late, e)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let oracle: Oracle = match args[1].as_str() {
        "norec" => sqlancer::norec,
        "tlp" => sqlancer::tlp,
        "plans" => sqlancer::plans,
        "order" => sqlancer::order,
        "setops" => sqlancer::setops,
        "cte" => sqlancer::ctes,
        "window" => sqlancer::windows,
        "storage" => sqlancer::storage,
        o => panic!("unknown oracle {o}"),
    };
    let first: u64 = args[2].parse().unwrap();
    let count: u64 = args[3].parse().unwrap();
    let flag = |name: &str| args.iter().position(|a| a == name).map(|i| args[i + 1].clone());
    let range = flag("--queries").map(|r| {
        let (lo, hi) = r.split_once('-').unwrap();
        (lo.parse::<usize>().unwrap().max(1), hi.parse::<usize>().unwrap())
    });
    let mutations: u32 = flag("--mutate").map_or(0, |k| k.parse().unwrap());
    let report: f64 = flag("--report").map_or(256.0, |k| k.parse().unwrap());
    let per_query = args.iter().any(|a| a == "--per-query");
    std::panic::set_hook(Box::new(|_| {}));
    let gen = case_queries();
    let start = Instant::now();
    let mut peaks = Vec::new();
    let mut fails = 0;
    for i in 0..count {
        let seed = first + i;
        let size = (seed % 101) as usize;
        let mut v = gen.generate(&mut Source::new(seed, size));
        if let Some(r) = range {
            extend(&mut v, seed, size, r);
        }
        if mutations > 0 {
            let mut g = Source::new(mix(seed, 10), size);
            for _ in 0..mutations {
                v = gen.mutate(&v, &mut g);
            }
        }
        let secs = 10.0 + (v.1.len() / 10) as f64;
        let (ms, sq, rs, late, e) = run(oracle, &v.0, &v.1, secs);
        fails += e.is_some() as u32;
        peaks.push(sq + rs);
        if sq + rs > report {
            println!("seed {seed} size {size} queries {} ms {ms:.0}{} sqlite {sq:.0} MB rust {rs:.0} MB", v.1.len(), if late { " LATE" } else { "" });
            if per_query {
                let mut each: Vec<(f64, f64, usize)> = v.1.iter().enumerate().map(|(k, q)| {
                    let (_, s, r, _, _) = run(oracle, &v.0, std::slice::from_ref(q), secs);
                    (s, r, k)
                }).collect();
                each.sort_by(|a, b| (b.0 + b.1).partial_cmp(&(a.0 + a.1)).unwrap());
                for (s, r, k) in each.iter().take(2) {
                    println!("  q{k} sqlite {s:.0} MB rust {r:.0} MB: {}", v.1[*k].sql());
                }
            }
        }
    }
    sqlancer::clear_storage();
    let mut s = peaks.clone();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p = |f: f64| s[((s.len() as f64 * f) as usize).min(s.len() - 1)];
    println!(
        "SUMMARY {} {first}..{} q {range:?} m {mutations}: peak MB p50 {:.1} p99 {:.1} p99.9 {:.1} max {:.0}; over 512: {}, over 1024: {}; {fails} failures; {:.1}s",
        args[1], first + count, p(0.5), p(0.99), p(0.999), s[s.len() - 1],
        s.iter().filter(|&&x| x > 512.0).count(), s.iter().filter(|&&x| x > 1024.0).count(),
        start.elapsed().as_secs_f64()
    );
}
