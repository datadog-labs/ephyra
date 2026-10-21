//! A driver for the oracle properties of `tests/sqlancer.rs` at scale,
//! without ephyra's per-test overhead: it draws inputs from the same
//! generator (`case_queries`) and runs exactly the same oracle functions
//! (`ephyra_postgresql::sqlancer`), seed after seed, on `threads` threads
//! (each with its own database).
//!
//! `cargo run --release --example sqlancer -- <norec|tlp|plans|order|setops|cte|generic|window|views|all> <first seed> <count> [threads]`
//!
//! Sizes cycle through 0..=100, as ephyra's runner draws them. A failure is
//! shrunk greedily with the generator's own shrinker (first candidate that
//! still fails the same way, until none does) and printed with the setup's
//! script, the statements and the queries. Ends with one summary line per
//! oracle: comparisons per variant and skips.

use std::{
    ops::ControlFlow,
    panic,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
    time::Instant,
};

use ephyra::{Gen, Source};
use ephyra_postgresql::{
    cases::Case,
    expr::Query,
    pg::close_session,
    probes::case_queries,
    sqlancer::{self, Tally},
};

type Oracle = fn(&Case, &[Query]) -> Result<Tally, String>;

const ORACLES: [(&str, Oracle); 9] = [
    ("norec", sqlancer::norec),
    ("tlp", sqlancer::tlp),
    ("plans", sqlancer::plans),
    ("order", sqlancer::order),
    ("setops", sqlancer::setops),
    ("cte", sqlancer::ctes),
    ("generic", sqlancer::generic),
    ("window", sqlancer::windows),
    ("views", sqlancer::views),
];

fn check(oracle: Oracle, v: &(Case, Vec<Query>)) -> Result<Tally, String> {
    match panic::catch_unwind(|| oracle(&v.0, &v.1)) {
        Ok(r) => r,
        Err(e) => Err(format!("panic: {:?}", e.downcast_ref::<String>().cloned().or(e.downcast_ref::<&str>().map(|s| s.to_string())))),
    }
}

/// A failure's kind: its first line with numbers masked (object ids and
/// TOAST value ids differ from run to run).
fn kind(e: &str) -> String {
    let mut out = String::new();
    let mut digits = false;
    for ch in e.lines().next().unwrap_or("").chars() {
        if ch.is_ascii_digit() {
            if !digits {
                out.push('#');
            }
            digits = true;
        } else {
            out.push(ch);
            digits = false;
        }
    }
    out
}

/// The failure shrunk greedily, with its message.
fn shrink(oracle: Oracle, v: (Case, Vec<Query>), e: String) -> ((Case, Vec<Query>), String) {
    let gen = case_queries();
    let kind_of = kind(&e);
    let (mut best, mut why) = (v, e);
    // At most two minutes per failure: a slow case shrinks part way.
    let budget = Instant::now() + std::time::Duration::from_secs(120);
    for _ in 0..2000 {
        if Instant::now() > budget {
            break;
        }
        let mut next = None;
        let _ = gen.shrink(&best, &mut |c| match check(oracle, &c) {
            Err(e) if kind(&e) == kind_of => {
                next = Some((c, e));
                ControlFlow::Break(())
            },
            _ => ControlFlow::Continue(()),
        });
        match next {
            Some((c, e)) => {
                best = c;
                why = e;
            },
            None => break,
        }
    }
    (best, why)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let which: Vec<(&str, Oracle)> = match args[1].as_str() {
        "all" => ORACLES.to_vec(),
        name => vec![*ORACLES.iter().find(|(n, _)| *n == name).unwrap_or_else(|| panic!("unknown oracle {name}"))],
    };
    let first: u64 = args[2].parse().unwrap();
    let count: u64 = args[3].parse().unwrap();
    let threads: u64 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(1);
    panic::set_hook(Box::new(|_| {}));
    for (name, oracle) in which {
        let start = Instant::now();
        let tally = Mutex::new(Tally::default());
        let (failures, empty, next) = (AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0));
        std::thread::scope(|scope| {
            for _ in 0..threads {
                scope.spawn(|| {
                    let gen = case_queries();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        if i >= count {
                            break;
                        }
                        let seed = first + i;
                        let v = gen.generate(&mut Source::new(seed, (seed % 101) as usize));
                        empty.fetch_add(u64::from(v.1.is_empty()), Ordering::Relaxed);
                        match check(oracle, &v) {
                            Ok(t) => tally.lock().unwrap().add(&t),
                            Err(e) => {
                                let n = failures.fetch_add(1, Ordering::Relaxed);
                                if n >= 10 {
                                    println!("FAIL {name} seed {seed} (not shrunk): {}", e.lines().next().unwrap_or(""));
                                    continue;
                                }
                                let (best, why) = shrink(oracle, v, e);
                                println!("FAIL {name} seed {seed}\n{why}\n{:?}", best.0);
                                for q in &best.1 {
                                    println!("  {};", q.sql());
                                }
                            },
                        }
                    }
                    close_session();
                });
            }
        });
        let t = tally.into_inner().unwrap();
        println!(
            "SUMMARY {name} seeds {first}..{}: {count} cases, {} failures, {} without queries, {} queries, {} comparisons {:?}, inapplicable {}, error skips {}, known-bug skips {}, timeouts {}, {:.1}s",
            first + count,
            failures.load(Ordering::Relaxed),
            empty.load(Ordering::Relaxed),
            t.queries,
            t.total(),
            t.comparisons,
            t.inapplicable,
            t.error_skips,
            t.known_skips,
            t.timeouts,
            start.elapsed().as_secs_f64()
        );
    }
}
