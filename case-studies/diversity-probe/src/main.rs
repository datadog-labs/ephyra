//! How diverse are the runner's inputs? 20000 fresh draws exactly as fresh
//! workers make them (worker seed `mix(run, id)`, input `n` from seed
//! `mix(worker_seed, n)` at size `n % 101`, under oracle `n % 7`), and:
//! duplicate inputs (same file, statements and queries), by size; distinct
//! coverage fingerprints (the set of SQLite counters an input reaches);
//! the spread of counters per input; the Jaccard similarity of random pairs;
//! and the discovery curve of distinct counters.

use std::{
    collections::{hash_map::DefaultHasher, BTreeMap, HashMap, HashSet},
    hash::{Hash, Hasher},
    time::{Duration, Instant},
};

use ephyra::{coverage, Gen, Source};
use ephyra_sqlite::{cases::Case, expr::Query, probes::case_queries, sqlancer::{self, Tally}};

type Oracle = fn(&Case, &[Query]) -> Result<Tally, String>;
const ORACLES: [Oracle; 7] = [sqlancer::norec, sqlancer::tlp, sqlancer::plans, sqlancer::order, sqlancer::setops, sqlancer::ctes, sqlancer::windows];

fn mix(seed: u64, index: u64) -> u64 {
    let mut z = seed.wrapping_add(index.wrapping_mul(0x9e37_79b9_7f4a_7c15));
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

fn hash<T: Hash>(t: &T) -> u64 {
    let mut h = DefaultHasher::new();
    t.hash(&mut h);
    h.finish()
}

fn main() {
    coverage::suppress_default_dump();
    std::panic::set_hook(Box::new(|_| {}));
    let gen = case_queries();
    let run = 1_791_558_610u64; // a real run's seed
    let workers = 13u64;
    let total = 20_000u64;
    // Warm-up, so once-per-process code is not credited to the first inputs.
    for i in 0..100 {
        let v = gen.generate(&mut Source::new(mix(7, i), i as usize));
        let _ = sqlancer::norec(&v.0, &v.1);
    }
    let mut inputs: HashMap<u64, u64> = HashMap::new();
    let mut by_size: BTreeMap<usize, (u64, HashSet<u64>)> = BTreeMap::new();
    let mut prints: HashMap<u64, u64> = HashMap::new();
    let mut sets: Vec<Vec<u32>> = Vec::new();
    let mut seen: HashSet<u32> = HashSet::new();
    let mut curve = Vec::new();
    let start = Instant::now();
    let mut k = 0u64;
    'outer: for n in 1.. {
        for id in 0..workers {
            if k == total {
                break 'outer;
            }
            k += 1;
            let s = mix(mix(run, id), n);
            let size = (n % 101) as usize;
            let v = gen.generate(&mut Source::new(s, size));
            let text = (v.0.file.0.clone(), v.0.statements.iter().map(|x| x.to_string()).collect::<Vec<_>>(), v.1.iter().map(|q| q.sql()).collect::<Vec<_>>());
            let h = hash(&text);
            *inputs.entry(h).or_default() += 1;
            let b = by_size.entry(size / 10 * 10).or_default();
            b.0 += 1;
            b.1.insert(h);
            sqlancer::set_deadline(Some(Instant::now() + Duration::from_secs(10)));
            let before = coverage::snapshot().unwrap();
            let _ = ORACLES[(n % 7) as usize](&v.0, &v.1);
            let delta = before.delta(&coverage::snapshot().unwrap());
            sqlancer::set_deadline(None);
            let mut cs: Vec<u32> = delta.hits().iter().map(|&(c, _)| c as u32).collect();
            cs.sort_unstable();
            *prints.entry(hash(&cs)).or_default() += 1;
            seen.extend(cs.iter().copied());
            if [1000, 2000, 5000, 10000, 20000].contains(&k) {
                curve.push((k, seen.len()));
            }
            sets.push(cs);
        }
    }
    let n = total as f64;
    println!("{total} inputs in {:.0} s", start.elapsed().as_secs_f64());
    let dup_inputs: u64 = inputs.values().filter(|&&c| c > 1).map(|c| c - 1).sum();
    println!("distinct inputs: {} ({:.2}% of draws repeat an earlier input)", inputs.len(), 100.0 * dup_inputs as f64 / n);
    println!("duplicates by size (size: draws, distinct):");
    for (s, (d, u)) in &by_size {
        println!("  {s:3}-{:<3} {d:6} draws, {:6} distinct ({:.1}% repeats)", s + 9, u.len(), 100.0 * (*d as f64 - u.len() as f64) / *d as f64);
    }
    let dup_prints: u64 = prints.values().filter(|&&c| c > 1).map(|c| c - 1).sum();
    let mut top: Vec<u64> = prints.values().copied().collect();
    top.sort_unstable_by(|a, b| b.cmp(a));
    println!(
        "distinct coverage fingerprints: {} ({:.1}% of inputs reach exactly the same counters as an earlier one; the 5 most common fingerprints cover {:?} inputs)",
        prints.len(),
        100.0 * dup_prints as f64 / n,
        &top[..5.min(top.len())]
    );
    let sizes: Vec<f64> = sets.iter().map(|s| s.len() as f64).collect();
    let mean = sizes.iter().sum::<f64>() / n;
    let sd = (sizes.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n).sqrt();
    println!("counters per input: mean {mean:.0}, sd {sd:.0} (coefficient of variation {:.2}), min {:.0}, max {:.0}", sd / mean, sizes.iter().cloned().fold(f64::INFINITY, f64::min), sizes.iter().cloned().fold(0.0, f64::max));
    let jac = |a: &[u32], b: &[u32]| {
        let (mut i, mut j, mut both) = (0, 0, 0usize);
        while i < a.len() && j < b.len() {
            match a[i].cmp(&b[j]) {
                std::cmp::Ordering::Less => i += 1,
                std::cmp::Ordering::Greater => j += 1,
                std::cmp::Ordering::Equal => {
                    both += 1;
                    i += 1;
                    j += 1;
                },
            }
        }
        both as f64 / (a.len() + b.len() - both).max(1) as f64
    };
    let pairs: Vec<f64> = (0..5000u64).map(|p| {
        let a = (mix(99, 2 * p) % total) as usize;
        let b = (mix(99, 2 * p + 1) % total) as usize;
        jac(&sets[a], &sets[b])
    }).collect();
    let pm = pairs.iter().sum::<f64>() / pairs.len() as f64;
    let psd = (pairs.iter().map(|x| (x - pm).powi(2)).sum::<f64>() / pairs.len() as f64).sqrt();
    let near = pairs.iter().filter(|&&x| x >= 0.99).count();
    println!("Jaccard of random pairs: mean {pm:.3}, sd {psd:.3}; {:.1}% of pairs at 0.99 or more", 100.0 * near as f64 / pairs.len() as f64);
    println!("discovery curve (inputs: distinct counters): {:?}", curve);
}
