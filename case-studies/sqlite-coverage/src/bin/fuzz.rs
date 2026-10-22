// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Does mutation pay? One arm of a coverage-guided loop over the oracle
//! inputs (a case and its queries), against the instrumented SQLite, for a
//! fixed wall-clock time:
//!
//! - `gen`: every input drawn afresh, as the scale driver does;
//! - `fifo`: ephyra's [`Fifo`] pool, unchanged (a fresh draw only while it is
//!   empty, else a mutant of its oldest seed);
//! - `mix`: a fresh draw and a mutant of the pool's oldest seed in turn.
//!
//! Each input runs under one oracle, the next in turn. Its feedback comes
//! from an [`Objective`], and an input with any (positive) feedback becomes a
//! seed, unless its file is over [`MAX_SEED_BYTES`] or the pool's files are
//! over [`POOL_BYTES`]. Two objectives:
//!
//! - `coverage` ([`Novelty`]): the (counter, hit-count bucket) features of
//!   SQLite's counters no earlier input reached;
//! - `combined` ([`Combined`]): the file-format counters the input's file
//!   reaches beyond its control (as the score counts them) that no earlier
//!   input reached, plus 1 when its metric vector extends the r-net
//!   ([`Net`]).
//!
//! Every arm computes all of them, whichever steers its pool, so their totals
//! compare arms. Every 30 seconds, and at the end, one line of
//! `<out>/<arm>-<seed>.tsv` gives the totals so far; failures go to
//! `<out>/<arm>-<seed>.failures`, unshrunk.
//!
//! `cargo run --release --bin fuzz -- <gen|fifo|mix> --objective <coverage|combined> --seed S --minutes M --out DIR`

use std::{
    collections::BTreeSet,
    fmt::Write as _,
    fs::{self, File},
    io::Write as _,
    panic::{self, AssertUnwindSafe},
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

use ephyra::{
    coverage,
    loops::{Directive, Fifo, SeedPool, Validity},
    Gen, Source,
};
use ephyra_sqlite::{
    cases::{case_metrics, Case},
    expr::Query,
    file_metrics,
    probes::{case_queries, probe_metrics},
    sqlancer::{self, Tally},
    FILE_FALLBACKS, QUERY_FALLBACKS,
};
use sqlite_coverage::{beyond, format_counters};

type Input = (Case, Vec<Query>);
type Oracle = fn(&Case, &[Query]) -> Result<Tally, String>;

const ORACLES: [(&str, Oracle); 7] = [
    ("norec", sqlancer::norec),
    ("tlp", sqlancer::tlp),
    ("plans", sqlancer::plans),
    ("order", sqlancer::order),
    ("setops", sqlancer::setops),
    ("cte", sqlancer::ctes),
    ("window", sqlancer::windows),
];

/// Seeds with a file over this many bytes are not kept (they cost memory and
/// time out of proportion).
const MAX_SEED_BYTES: usize = 2 << 20;

/// No seed is kept while the pool's files add up to more than this (ephyra's
/// pools cannot evict).
const POOL_BYTES: usize = 3 << 30;

/// Fresh draws that fix the metric vectors' scale and the net's radius, the
/// same for every arm.
const REFERENCE: u64 = 2000;

/// An input's feedback from the counters it hit under its oracle: positive
/// when it is worth keeping as a seed, higher when more so.
trait Objective {
    fn observe(&mut self, input: &Input, hits: &[(usize, u64)]) -> f64;
}

/// AFL's hit-count buckets: 1, 2, 3, 4-7, 8-15, 16-31, 32-127, 128+.
fn bucket(hits: u64) -> u8 {
    match hits {
        0 => 0,
        1 => 1,
        2 => 1 << 1,
        3 => 1 << 2,
        4..=7 => 1 << 3,
        8..=15 => 1 << 4,
        16..=31 => 1 << 5,
        32..=127 => 1 << 6,
        _ => 1 << 7,
    }
}

/// The (counter, bucket) features an input reached that no earlier one did.
struct Novelty {
    seen: Vec<u8>,
}

impl Objective for Novelty {
    fn observe(&mut self, _input: &Input, hits: &[(usize, u64)]) -> f64 {
        let mut new = 0;
        for &(c, h) in hits {
            if c >= self.seen.len() {
                self.seen.resize(c + 1, 0);
            }
            let b = bucket(h);
            if self.seen[c] & b == 0 {
                self.seen[c] |= b;
                new += 1;
            }
        }
        f64::from(new)
    }
}

impl Novelty {
    fn counters(&self, within: Option<&BTreeSet<usize>>) -> usize {
        self.seen
            .iter()
            .enumerate()
            .filter(|&(c, &b)| b != 0 && within.is_none_or(|w| w.contains(&c)))
            .count()
    }

    fn features(&self) -> u32 {
        self.seen.iter().map(|b| b.count_ones()).sum()
    }
}

/// The input's metric vector: [`probe_metrics`], [`case_metrics`] and
/// [`file_metrics`], each name once.
fn metrics(v: &Input) -> Vec<f64> {
    let mut names = BTreeSet::new();
    probe_metrics(v, 0)
        .into_iter()
        .chain(case_metrics(&v.0, 0))
        .chain(file_metrics(&v.0.file, 0))
        .filter(|(n, _)| names.insert(*n))
        .map(|(_, x)| x)
        .collect()
}

/// Metric vectors scaled as the reference sample fixes: `sign(x) log(1 + |x|)`,
/// then z-scored per dimension (dimensions constant in the sample dropped).
struct Scale {
    mean: Vec<f64>,
    sd: Vec<f64>,
}

impl Scale {
    fn of(sample: &[Vec<f64>]) -> Scale {
        let n = sample.len() as f64;
        let dims = sample[0].len();
        let t: Vec<Vec<f64>> = sample.iter().map(|v| v.iter().map(|&x| squash(x)).collect()).collect();
        let mean: Vec<f64> = (0..dims).map(|d| t.iter().map(|v| v[d]).sum::<f64>() / n).collect();
        let sd = (0..dims).map(|d| (t.iter().map(|v| (v[d] - mean[d]).powi(2)).sum::<f64>() / n).sqrt()).collect();
        Scale { mean, sd }
    }

    fn apply(&self, v: &[f64]) -> Vec<f32> {
        v.iter()
            .zip(self.mean.iter().zip(&self.sd))
            .filter(|(_, (_, &sd))| sd > 0.0)
            .map(|(&x, (&m, &sd))| ((squash(x) - m) / sd) as f32)
            .collect()
    }
}

fn squash(x: f64) -> f64 {
    x.signum() * x.abs().ln_1p()
}

fn distance(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum::<f32>().sqrt()
}

/// A greedy r-net over scaled metric vectors: a vector joins when no member
/// is within `r`. Its size measures how much of the metric space the inputs
/// so far cover, without bins. `r` is the median nearest-neighbour distance
/// within the reference sample.
struct Net {
    r: f32,
    points: Vec<Vec<f32>>,
}

impl Net {
    fn of(reference: &[Vec<f32>]) -> Net {
        let mut nearest: Vec<f32> = reference
            .iter()
            .enumerate()
            .map(|(i, a)| {
                reference.iter().enumerate().filter(|&(j, _)| j != i).map(|(_, b)| distance(a, b)).fold(f32::INFINITY, f32::min)
            })
            .collect();
        nearest.sort_by(f32::total_cmp);
        Net { r: nearest[nearest.len() / 2], points: Vec::new() }
    }

    /// Whether `v` joins the net (and then adds it).
    fn extend(&mut self, v: Vec<f32>) -> bool {
        if self.points.iter().any(|p| distance(p, &v) <= self.r) {
            return false;
        }
        self.points.push(v);
        true
    }
}

/// File-format counters beyond the control no earlier input reached, plus 1
/// when the input's metric vector extends the net.
struct Combined {
    format: BTreeSet<usize>,
    beyond: BTreeSet<usize>,
    scale: Scale,
    net: Net,
}

impl Combined {
    fn new(format: BTreeSet<usize>, gen: &impl Gen<Value = Input>) -> Combined {
        let sample: Vec<Vec<f64>> =
            (0..REFERENCE).map(|i| metrics(&gen.generate(&mut Source::new(0xc0ff_ee00 + i, (i % 101) as usize)))).collect();
        let scale = Scale::of(&sample);
        let scaled: Vec<Vec<f32>> = sample.iter().map(|v| scale.apply(v)).collect();
        let net = Net::of(&scaled);
        Combined { format, beyond: BTreeSet::new(), scale, net }
    }
}

impl Objective for Combined {
    fn observe(&mut self, input: &Input, _hits: &[(usize, u64)]) -> f64 {
        let mut new = 0.0;
        for c in beyond(&self.format, &input.0) {
            if self.beyond.insert(c) {
                new += 1.0;
            }
        }
        if self.net.extend(self.scale.apply(&metrics(input))) {
            new += 1.0;
        }
        new
    }
}

/// SplitMix64.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Arm {
    Gen,
    Fifo,
    Mix,
}

/// Totals for one kind of input (fresh or mutant).
#[derive(Default)]
struct Kind {
    inputs: u64,
    novel: u64,
    features: u64,
    secs: f64,
}

fn panic_message(e: Box<dyn std::any::Any + Send>) -> String {
    e.downcast_ref::<String>().cloned().or(e.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default()
}

fn main() {
    coverage::suppress_default_dump();
    panic::set_hook(Box::new(|_| {}));
    let args: Vec<String> = std::env::args().collect();
    let arm = match args[1].as_str() {
        "gen" => Arm::Gen,
        "fifo" => Arm::Fifo,
        "mix" => Arm::Mix,
        a => panic!("unknown arm {a}"),
    };
    let value = |flag: &str| args.iter().position(|a| a == flag).map(|i| args[i + 1].clone());
    let seed: u64 = value("--seed").map_or(0, |s| s.parse().unwrap());
    let minutes: f64 = value("--minutes").map_or(10.0, |s| s.parse().unwrap());
    let out = value("--out").unwrap_or_else(|| ".".into());
    let steer_combined = match value("--objective").as_deref() {
        None | Some("coverage") => false,
        Some("combined") => true,
        Some(o) => panic!("unknown objective {o}"),
    };
    fs::create_dir_all(&out).unwrap();
    let name = format!("{out}/{}-{seed}", args[1]);
    let mut tsv = File::create(format!("{name}.tsv")).unwrap();
    let mut failures = File::create(format!("{name}.failures")).unwrap();
    writeln!(
        tsv,
        "secs\texecs\tcorpus\tcounters\tformat_counters\tfeatures\tbeyond\tnet\ttoo_big\tfull\tfailures\tgen_panics\t\
         file_fallbacks\tquery_fallbacks\tfresh\tfresh_novel\tfresh_features\tfresh_secs\tmutants\tmutant_novel\t\
         mutant_features\tmutant_secs"
    )
    .unwrap();

    let format = format_counters();
    let gen = case_queries();
    let mut rng = Rng(seed);
    let mut pool: Fifo<Input, Validity> = Fifo::new();
    let mut objective = Novelty { seen: Vec::new() };
    let mut combined = Combined::new(format.clone(), &gen);
    let (mut too_big, mut full) = (0u64, 0u64);
    let (mut fresh, mut mutant) = (Kind::default(), Kind::default());
    let (mut execs, mut failed, mut gen_panics) = (0u64, 0u64, 0u64);
    let start = Instant::now();
    let deadline = start + Duration::from_secs_f64(minutes * 60.0);
    let mut next_line = start;
    loop {
        let now = Instant::now();
        let done = now >= deadline;
        if now >= next_line || done {
            let mut line = String::new();
            write!(
                line,
                "{:.0}\t{execs}\t{}\t{}\t{}\t{}\t{}\t{}\t{too_big}\t{full}\t{failed}\t{gen_panics}\t{}\t{}",
                start.elapsed().as_secs_f64(),
                pool.seeds().count(),
                objective.counters(None),
                objective.counters(Some(&format)),
                objective.features(),
                combined.beyond.len(),
                combined.net.points.len(),
                FILE_FALLBACKS.load(Ordering::Relaxed),
                QUERY_FALLBACKS.load(Ordering::Relaxed),
            )
            .unwrap();
            for k in [&fresh, &mutant] {
                write!(line, "\t{}\t{}\t{}\t{:.1}", k.inputs, k.novel, k.features, k.secs).unwrap();
            }
            writeln!(tsv, "{line}").unwrap();
            tsv.flush().unwrap();
            next_line = now + Duration::from_secs(30);
        }
        if done {
            break;
        }

        let t = Instant::now();
        let mut g = Source::new(rng.next(), (execs % 101) as usize);
        let parent = match arm {
            Arm::Gen => None,
            Arm::Fifo => match pool.sample() {
                Directive::Generate => None,
                Directive::Mutate(seed) => Some(seed),
            },
            Arm::Mix if execs % 2 == 0 => None,
            Arm::Mix => match pool.sample() {
                Directive::Generate => None,
                Directive::Mutate(seed) => Some(seed),
            },
        };
        let mutated = parent.is_some();
        let drawn = panic::catch_unwind(AssertUnwindSafe(|| match &parent {
            Some(p) => gen.mutate(p, &mut g),
            None => gen.generate(&mut g),
        }));
        let input = match drawn {
            Ok(input) => input,
            Err(e) => {
                gen_panics += 1;
                writeln!(failures, "generator panic (mutated {mutated}): {}", panic_message(e)).unwrap();
                if mutated {
                    pool.revise();
                }
                continue;
            },
        };

        let (oracle_name, oracle) = ORACLES[(execs % ORACLES.len() as u64) as usize];
        let before = coverage::snapshot().expect("an instrumented SQLite");
        let verdict = match panic::catch_unwind(|| oracle(&input.0, &input.1)) {
            Ok(r) => r,
            Err(e) => Err(format!("panic: {}", panic_message(e))),
        };
        let delta = before.delta(&coverage::snapshot().expect("an instrumented SQLite"));
        execs += 1;
        let covered = objective.observe(&input, delta.hits());
        let both = combined.observe(&input, delta.hits());
        let new = if steer_combined { both } else { covered };
        if let Err(e) = verdict {
            failed += 1;
            if failed <= 50 {
                writeln!(failures, "FAIL exec {execs} {oracle_name} (mutated {mutated})\n{e}\n{:?}\n{:?}\n", input.0, input.1).unwrap();
            }
        }
        let k = if mutated { &mut mutant } else { &mut fresh };
        k.inputs += 1;
        k.secs += t.elapsed().as_secs_f64();
        if new > 0.0 {
            k.novel += 1;
            k.features += new as u64;
            if input.0.file.0.len() > MAX_SEED_BYTES {
                too_big += 1;
            } else if arm != Arm::Gen && pool.seeds().map(|s| s.input.0.file.0.len()).sum::<usize>() > POOL_BYTES {
                full += 1;
            } else if arm != Arm::Gen {
                pool.invest(input, Validity { valid: true, score: new });
            }
        } else if mutated {
            pool.revise();
        }
    }
}
