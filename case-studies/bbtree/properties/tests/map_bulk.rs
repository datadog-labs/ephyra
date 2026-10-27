// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! bbtree's maps at sizes its fuzz targets rarely reach: bulk inserts of
//! up to 10^5 entries, then removal-heavy phases (strided removes,
//! pop_first/pop_last runs, retain, extract_if over a range, split_off and
//! append), so that merges, steals, root lowering and shared-prefix
//! changes happen at depth 3 and 4. Each phase is compared whole against
//! std's `BTreeMap`; built with `--cfg fuzzing`, the tree's own invariant
//! checks run after every phase too.

#![allow(unexpected_cfgs)]

use std::collections::BTreeMap;
use std::fmt::Debug;

use bbtree::unsync::BBTreeMap;
use bbtree::{Auto, Configured, InlineKeyStrategy, NodeConfig};
use ephyra::{BoxGen, Config, GenExt, Report, TestResult, check, gens};

#[derive(Clone, Debug, PartialEq)]
enum Phase {
    /// `count` keys stepping from `start` by `stride`.
    Insert { start: u32, stride: u32, count: u32 },
    Remove { start: u32, stride: u32, count: u32 },
    PopFirst(u32),
    PopLast(u32),
    /// Keeps keys whose number is not ≡ `rem` mod `modulus`.
    Retain { modulus: u32, rem: u32 },
    /// Extracts keys in `lo..hi` whose number is ≡ `rem` mod `modulus`, taking
    /// only the first `take` the iterator gives (the rest stay).
    ExtractIf { lo: u32, hi: u32, modulus: u32, rem: u32, take: u32 },
    /// Splits at `at`, and appends the split part back (or keeps it apart
    /// and drops it, if `drop`).
    SplitOff { at: u32, drop: bool },
    /// Removes a contiguous run.
    RemoveRun { lo: u32, hi: u32 },
}

fn phase() -> BoxGen<Phase> {
    let n = gens::int(0u32..=120_000);
    let stride = gens::frequency(vec![(4, gens::int(1u32..=3).boxed()), (2, gens::int(1u32..=64).boxed()), (1, gens::int(1u32..=5000).boxed())]);
    let count = gens::frequency(vec![(2, gens::int(0u32..=300).boxed()), (3, gens::int(0u32..=20_000).boxed()), (1, gens::int(0u32..=100_000).boxed())]);
    gens::frequency(vec![
        (5, (n.clone(), stride.clone(), count.clone()).map(|(start, stride, count)| Phase::Insert { start, stride, count }).boxed()),
        (5, (n.clone(), stride, count.clone()).map(|(start, stride, count)| Phase::Remove { start, stride, count }).boxed()),
        (2, count.clone().map(Phase::PopFirst).boxed()),
        (2, count.clone().map(Phase::PopLast).boxed()),
        (1, (gens::int(1u32..=7), gens::int(0u32..=6)).map(|(modulus, rem)| Phase::Retain { modulus, rem: rem % modulus }).boxed()),
        (
            1,
            (n.clone(), n.clone(), gens::int(1u32..=5), gens::int(0u32..=4), count)
                .map(|(a, b, modulus, rem, take)| Phase::ExtractIf { lo: a.min(b), hi: a.max(b), modulus, rem: rem % modulus, take })
                .boxed(),
        ),
        (1, (n.clone(), gens::bool()).map(|(at, drop)| Phase::SplitOff { at, drop }).boxed()),
        (2, (n.clone(), n).map(|(a, b)| Phase::RemoveRun { lo: a.min(b), hi: a.max(b) }).boxed()),
    ])
    .boxed()
}

fn case() -> BoxGen<Vec<Phase>> {
    gens::vec(phase()).len(1..=14).boxed()
}

/// Keys of each kind made from a number, ordered as the numbers are.
trait Key: Ord + Clone + Debug {
    fn of(n: u32) -> Self;
}
impl Key for u64 {
    fn of(n: u32) -> Self {
        n as u64
    }
}
impl Key for i128 {
    /// Spread across the sign, with a large shared high part.
    fn of(n: u32) -> Self {
        (n as i128 - 60_000) * (1 << 70) + 12345
    }
}
impl Key for String {
    /// A long shared prefix, then the number, zero-padded.
    fn of(n: u32) -> Self {
        format!("tenant/0042/user/{n:08}")
    }
}
impl Key for Vec<u8> {
    /// Variable length: the number's big-endian bytes without leading
    /// zeros would misorder, so a length byte goes first.
    fn of(n: u32) -> Self {
        let be = n.to_be_bytes();
        let skip = be.iter().take_while(|&&b| b == 0).count();
        let mut k = vec![b'k', (4 - skip) as u8];
        k.extend_from_slice(&be[skip..]);
        k
    }
}
impl Key for [u8; 16] {
    fn of(n: u32) -> Self {
        let mut k = [0xAB; 16];
        k[12..].copy_from_slice(&n.to_be_bytes());
        k
    }
}

fn run<K: Key, S: InlineKeyStrategy<K> + Default>(phases: &[Phase]) -> TestResult {
    let mut map = BBTreeMap::<K, u64, S>::with_strategy(S::default());
    let mut model: BTreeMap<K, u64> = BTreeMap::new();
    let (mut peak, mut removed_at_depth) = (0usize, false);
    for (i, phase) in phases.iter().enumerate() {
        removed_at_depth |= model.len() >= 10_000 && !matches!(phase, Phase::Insert { .. });
        let tag = i as u64;
        match *phase {
            Phase::Insert { start, stride, count } => {
                for j in 0..count {
                    let k = K::of(start.wrapping_add(j.wrapping_mul(stride)) % 130_000);
                    let v = tag << 32 | j as u64;
                    if map.insert(k.clone(), v) != model.insert(k, v) {
                        return TestResult::Fail(format!("phase {i} {phase:?}: insert {j} returned differently"));
                    }
                }
            }
            Phase::Remove { start, stride, count } => {
                for j in 0..count {
                    let k = K::of(start.wrapping_add(j.wrapping_mul(stride)) % 130_000);
                    if map.remove(&k) != model.remove(&k) {
                        return TestResult::Fail(format!("phase {i} {phase:?}: remove {j} ({k:?}) returned differently"));
                    }
                }
            }
            Phase::PopFirst(count) => {
                for j in 0..count {
                    let (a, b) = (map.pop_first(), model.pop_first());
                    if a != b {
                        return TestResult::Fail(format!("phase {i} {phase:?}: pop {j}: {a:?} vs {b:?}"));
                    }
                    if b.is_none() {
                        break;
                    }
                }
            }
            Phase::PopLast(count) => {
                for j in 0..count {
                    let (a, b) = (map.pop_last(), model.pop_last());
                    if a != b {
                        return TestResult::Fail(format!("phase {i} {phase:?}: pop {j}: {a:?} vs {b:?}"));
                    }
                    if b.is_none() {
                        break;
                    }
                }
            }
            Phase::Retain { modulus, rem } => {
                let keep = |v: &u64| (*v as u32) % modulus != rem;
                map.retain(|_, v| keep(v));
                model.retain(|_, v| keep(v));
            }
            Phase::ExtractIf { lo, hi, modulus, rem, take } => {
                let pick = |v: &u64| (*v as u32) % modulus == rem;
                let ours: Vec<_> = map.extract_if(K::of(lo)..K::of(hi), |_, v| pick(v)).take(take as usize).collect();
                let want: Vec<_> = model.extract_if(K::of(lo)..K::of(hi), |_, v| pick(v)).take(take as usize).collect();
                if ours != want {
                    return TestResult::Fail(format!("phase {i} {phase:?}: extracted {} vs {}", ours.len(), want.len()));
                }
            }
            Phase::SplitOff { at, drop } => {
                let k = K::of(at);
                let mut ours = map.split_off(&k);
                let mut want = model.split_off(&k);
                if !ours.iter().eq(want.iter()) || !map.iter().eq(model.iter()) {
                    return TestResult::Fail(format!("phase {i} {phase:?}: halves differ ({}+{} vs {}+{})", map.len(), ours.len(), model.len(), want.len()));
                }
                #[cfg(fuzzing)]
                ours.check_invariants();
                if !drop {
                    map.append(&mut ours);
                    model.append(&mut want);
                }
            }
            Phase::RemoveRun { lo, hi } => {
                for n in lo..hi.min(lo + 50_000) {
                    let k = K::of(n);
                    if map.remove(&k) != model.remove(&k) {
                        return TestResult::Fail(format!("phase {i} {phase:?}: remove {n} returned differently"));
                    }
                }
            }
        }
        peak = peak.max(model.len());
        if map.len() != model.len() {
            return TestResult::Fail(format!("after phase {i} {phase:?}: len {} vs {}", map.len(), model.len()));
        }
        if let Some(at) = map.iter().zip(model.iter()).position(|(a, b)| a != b) {
            return TestResult::Fail(format!("after phase {i} {phase:?}: entries differ at {at}"));
        }
        if !map.iter().rev().eq(model.iter().rev()) {
            return TestResult::Fail(format!("after phase {i} {phase:?}: reverse iteration differs"));
        }
        if map.first_key_value() != model.first_key_value() || map.last_key_value() != model.last_key_value() {
            return TestResult::Fail(format!("after phase {i} {phase:?}: first/last differ"));
        }
        for (k, v) in model.iter().step_by((model.len() / 200).max(1)) {
            if map.get(k) != Some(v) {
                return TestResult::Fail(format!("after phase {i} {phase:?}: get({k:?}) {:?}, want {v}", map.get(k)));
            }
        }
        #[cfg(fuzzing)]
        {
            map.check_invariants();
            map.check_prefix_lens();
        }
    }
    ephyra::classify(peak >= 10_000, "peak >= 10k entries");
    ephyra::classify(peak >= 50_000, "peak >= 50k entries");
    ephyra::classify(removed_at_depth, "removals from >= 10k entries");
    TestResult::Pass
}

struct Narrow;
impl NodeConfig for Narrow {
    const INTERNAL_PAGE_SIZE: usize = 64;
    const PLAIN_INTERNAL_PAGE_SIZE: usize = 64;
}
struct Wide;
impl NodeConfig for Wide {
    const LEAF_PAGE_SIZE: usize = 4096;
    const INTERNAL_PAGE_SIZE: usize = 4096;
    const PLAIN_INTERNAL_PAGE_SIZE: usize = 4096;
}

fn config(name: &str) -> Config {
    Config::default().name(name).tests(60)
}

fn print(report: Report) -> Report {
    eprintln!("{report}");
    report
}

macro_rules! bulk {
    ($name:ident, $k:ty, $s:ty) => {
        #[test]
        fn $name() {
            print(check(&config(stringify!($name)), case(), |phases| run::<$k, $s>(&phases))).assert_passed();
        }
    };
}

bulk!(bulk_u64, u64, Auto);
bulk!(bulk_i128, i128, Auto);
bulk!(bulk_string, String, Auto);
bulk!(bulk_bytes, Vec<u8>, Auto);
bulk!(bulk_arr16, [u8; 16], Auto);
bulk!(bulk_u64_narrow, u64, Configured<Auto, Narrow>);
bulk!(bulk_string_narrow, String, Configured<Auto, Narrow>);
bulk!(bulk_string_inline_narrow, String, Configured<bbtree::WithInlineKey, Narrow>);
bulk!(bulk_bytes_wide, Vec<u8>, Configured<Auto, Wide>);
bulk!(bulk_i128_wide, i128, Configured<Auto, Wide>);
