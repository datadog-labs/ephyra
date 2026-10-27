// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! bbtree's single-threaded map at sizes its fuzz targets rarely reach:
//! bulk inserts of up to 10^5 entries, then removal-heavy phases (strided
//! removes, pop_first/pop_last runs, retain, extract_if over a range,
//! split_off and append), so that merges, steals, root lowering and
//! shared-prefix changes happen at depth 3 and 4. Each phase is compared
//! whole against std's `BTreeMap`; built with `--cfg fuzzing`, the tree's
//! own invariant checks run after every phase too.

use std::collections::BTreeMap;
use std::fmt::Debug;

use bbtree::unsync::BBTreeMap;
use bbtree::{InlineKeyStrategy, NodeConfig};
use ephyra::{BoxGen, GenExt, classify, gens};
use serde::{Deserialize, Serialize};

use crate::common::{Fail, ensure, fail};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Phase {
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
    let stride =
        gens::frequency(vec![(4, gens::int(1u32..=3).boxed()), (2, gens::int(1u32..=64).boxed()), (1, gens::int(1u32..=5000).boxed())]);
    let count = gens::frequency(vec![
        (2, gens::int(0u32..=300).boxed()),
        (3, gens::int(0u32..=20_000).boxed()),
        (1, gens::int(0u32..=100_000).boxed()),
    ]);
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

pub type Case = Vec<Phase>;

pub fn case() -> BoxGen<Case> {
    gens::vec(phase()).len(1..=14).boxed()
}

/// Keys of each kind made from a number, ordered as the numbers are.
pub trait Key: Ord + Clone + Debug {
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
    /// Variable length: a length byte first, so that byte order is number order.
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

/// The tree's own invariant checks (compiled under `--cfg fuzzing`); a
/// panic in them is a broken invariant.
#[allow(unused_variables)]
fn invariants<K: Key, S: InlineKeyStrategy<K>>(map: &BBTreeMap<K, u64, S>, at: &str) -> Result<(), Fail> {
    #[cfg(fuzzing)]
    {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            map.check_invariants();
            map.check_prefix_lens();
        }));
        if let Err(p) = r {
            let msg = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()));
            return Err(fail("bulkmap__invariant__check", format!("{at}: {msg:?}")));
        }
    }
    Ok(())
}

pub fn run<K: Key, S: InlineKeyStrategy<K> + Default>(phases: &Case) -> Result<(), Fail> {
    let mut map = BBTreeMap::<K, u64, S>::with_strategy(S::default());
    let mut model: BTreeMap<K, u64> = BTreeMap::new();
    let (mut peak, mut removed_at_depth) = (0usize, false);
    for (i, phase) in phases.iter().enumerate() {
        removed_at_depth |= model.len() >= 10_000 && !matches!(phase, Phase::Insert { .. });
        let tag = i as u64;
        let at = |why: String| format!("phase {i} {phase:?}: {why}");
        match *phase {
            Phase::Insert { start, stride, count } => {
                for j in 0..count {
                    let k = K::of(start.wrapping_add(j.wrapping_mul(stride)) % 130_000);
                    let v = tag << 32 | j as u64;
                    let (a, b) = (map.insert(k.clone(), v), model.insert(k, v));
                    ensure(a == b, "bulkmap__mismatch__insert", || at(format!("insert {j}: {a:?} vs {b:?}")))?;
                }
            }
            Phase::Remove { start, stride, count } => {
                for j in 0..count {
                    let k = K::of(start.wrapping_add(j.wrapping_mul(stride)) % 130_000);
                    let (a, b) = (map.remove(&k), model.remove(&k));
                    ensure(a == b, "bulkmap__mismatch__remove", || at(format!("remove {j} ({k:?}): {a:?} vs {b:?}")))?;
                }
            }
            Phase::PopFirst(count) => {
                for j in 0..count {
                    let (a, b) = (map.pop_first(), model.pop_first());
                    ensure(a == b, "bulkmap__mismatch__pop_first", || at(format!("pop {j}: {a:?} vs {b:?}")))?;
                    if b.is_none() {
                        break;
                    }
                }
            }
            Phase::PopLast(count) => {
                for j in 0..count {
                    let (a, b) = (map.pop_last(), model.pop_last());
                    ensure(a == b, "bulkmap__mismatch__pop_last", || at(format!("pop {j}: {a:?} vs {b:?}")))?;
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
                ensure(ours == want, "bulkmap__mismatch__extract_if", || at(format!("extracted {} vs {}", ours.len(), want.len())))?;
            }
            Phase::SplitOff { at: split, drop } => {
                let k = K::of(split);
                let mut ours = map.split_off(&k);
                let mut want = model.split_off(&k);
                ensure(ours.iter().eq(want.iter()) && map.iter().eq(model.iter()), "bulkmap__mismatch__split_off", || {
                    at(format!("halves differ ({}+{} vs {}+{})", map.len(), ours.len(), model.len(), want.len()))
                })?;
                invariants(&ours, &at("the split-off half".into()))?;
                if !drop {
                    map.append(&mut ours);
                    model.append(&mut want);
                }
            }
            Phase::RemoveRun { lo, hi } => {
                for n in lo..hi.min(lo + 50_000) {
                    let k = K::of(n);
                    let (a, b) = (map.remove(&k), model.remove(&k));
                    ensure(a == b, "bulkmap__mismatch__remove_run", || at(format!("remove {n}: {a:?} vs {b:?}")))?;
                }
            }
        }
        peak = peak.max(model.len());
        ensure(map.len() == model.len(), "bulkmap__mismatch__len", || at(format!("after it: len {} vs {}", map.len(), model.len())))?;
        if let Some(pos) = map.iter().zip(model.iter()).position(|(a, b)| a != b) {
            return Err(fail("bulkmap__mismatch__iter", at(format!("after it: entries differ at {pos}"))));
        }
        ensure(map.iter().rev().eq(model.iter().rev()), "bulkmap__mismatch__iter_rev", || at("after it: reverse iteration differs".into()))?;
        ensure(
            map.first_key_value() == model.first_key_value() && map.last_key_value() == model.last_key_value(),
            "bulkmap__mismatch__first_last",
            || at("after it: first/last differ".into()),
        )?;
        for (k, v) in model.iter().step_by((model.len() / 200).max(1)) {
            ensure(map.get(k) == Some(v), "bulkmap__mismatch__get", || at(format!("after it: get({k:?}) {:?}, want {v}", map.get(k))))?;
        }
        invariants(&map, &at("after it".into()))?;
    }
    classify(peak >= 10_000, "peak_10k");
    classify(peak >= 50_000, "peak_50k");
    classify(removed_at_depth, "removals_from_10k");
    Ok(())
}

pub struct Narrow;
impl NodeConfig for Narrow {
    const INTERNAL_PAGE_SIZE: usize = 64;
    const PLAIN_INTERNAL_PAGE_SIZE: usize = 64;
}
pub struct Wide;
impl NodeConfig for Wide {
    const LEAF_PAGE_SIZE: usize = 4096;
    const INTERNAL_PAGE_SIZE: usize = 4096;
    const PLAIN_INTERNAL_PAGE_SIZE: usize = 4096;
}
