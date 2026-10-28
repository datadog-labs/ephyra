// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! `bbtree::BBTreeMap` shared by 2 to 6 threads on trees preloaded to tens
//! of thousands of entries: writers insert, update, remove (single and
//! batched, removal-heavy so that leaves merge and the root lowers) while
//! readers get, get_many, scan, scan_rev and walk range_cloned. Every value
//! names its key, so a reader that lands on the wrong slot, a torn entry or
//! a stale node sees a value of another key; scans hand out keys in order,
//! within range, each once. After the join, keys one thread alone wrote
//! hold what it left, and the tree's invariants hold (`--cfg fuzzing`).
//! Nondeterministic: repro tries several times.

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::ops::{Bound, ControlFlow};
use std::sync::Barrier;

use bbtree::{BBTreeMap, InlineKeyStrategy};
use ephyra::{BoxGen, GenExt, gens};
use serde::{Deserialize, Serialize};

use crate::common::{Fail, ensure, fail, spawn_in};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Op {
    Insert(u32),
    Remove(u32),
    RemoveRun(u32, u16),
    InsertMany([u32; 4]),
    Update(u32),
    Get(u32),
    GetMany([u32; 4]),
    Scan(u32, u32, bool),
    Range(u32, u32),
}

fn op(space: u32) -> BoxGen<Op> {
    // Now and then the lowest numbers: the keys outside the shared prefix
    // under `SHRINK`, and scans from below it.
    let k = gens::frequency(vec![(19, gens::int(0u32..space).boxed()), (1, gens::int(0u32..40).boxed())]).boxed();
    let k4 = (k.clone(), k.clone(), k.clone(), k.clone()).map(|(a, b, c, d)| [a, b, c, d]).boxed();
    gens::frequency(vec![
        (4, k.clone().map(Op::Insert).boxed()),
        (5, k.clone().map(Op::Remove).boxed()),
        (1, (k.clone(), gens::int(1u16..=2000)).map(|(a, n)| Op::RemoveRun(a, n)).boxed()),
        (2, k4.clone().map(Op::InsertMany).boxed()),
        (2, k.clone().map(Op::Update).boxed()),
        (3, k.clone().map(Op::Get).boxed()),
        (2, k4.map(Op::GetMany).boxed()),
        (1, (k.clone(), k.clone(), gens::bool()).map(|(a, b, r)| Op::Scan(a.min(b), a.max(b), r)).boxed()),
        (1, (k.clone(), k).map(|(a, b)| Op::Range(a.min(b), a.max(b))).boxed()),
    ])
    .boxed()
}

const SPACE: u32 = 60_000;

/// Key space actually used (ops are taken modulo it), preload, one plan
/// per thread.
pub type Case = (u32, u32, Vec<Vec<Op>>);

pub fn case() -> BoxGen<Case> {
    (
        // Tiny spaces empty the tree now and then: the next insert starts the
        // shared prefix anew from its key, and the inserts after it shrink it.
        gens::frequency(vec![(1, gens::int(2u32..=64).boxed()), (2, gens::int(64u32..=2000).boxed()), (4, gens::int(2000u32..=SPACE).boxed())]),
        gens::int(0u32..=SPACE),
        gens::vec(gens::vec(op(SPACE)).len(0..=3000)).len(2..=6),
    )
        .boxed()
}

pub trait Key: Ord + Clone + Debug + Send + Sync + 'static {
    /// With `shrink`, the lowest numbers name keys outside the tree-wide
    /// shared prefix (D50), in steps (see `outside`); `of` stays monotone in
    /// `n` either way, so ranges by number are ranges by key.
    fn of(n: u32, shrink: bool) -> Self;
    fn number(&self, shrink: bool) -> u32;
    /// Keys never preloaded under `shrink`: the first thread to insert one
    /// shrinks the shared prefix while the others run.
    fn outside(n: u32) -> bool;
}
impl Key for u64 {
    // Under shrink: 0..8 share 2 bytes with the rest, 8..16 share 3, the
    // rest share 5.
    fn of(n: u32, shrink: bool) -> Self {
        match (shrink, n) {
            (false, _) => (0xAB << 40) + n as u64 * 3,
            (true, 0..8) => n as u64,
            (true, 8..16) => (0xAB << 40) + n as u64,
            (true, _) => (0xAB << 40) + (1 << 32) + n as u64 * 3,
        }
    }
    fn number(&self, shrink: bool) -> u32 {
        if !shrink {
            ((self - (0xAB << 40)) / 3) as u32
        } else if *self < 8 {
            *self as u32
        } else if *self < (0xAB << 40) + (1 << 32) {
            (self - (0xAB << 40)) as u32
        } else {
            ((self - (0xAB << 40) - (1 << 32)) / 3) as u32
        }
    }
    fn outside(n: u32) -> bool {
        n < 16
    }
}
impl Key for String {
    // Under shrink: "t0".."t7" share "t" with the rest, then "tenant/",
    // "tenant/7/", "tenant/7/key/".
    fn of(n: u32, shrink: bool) -> Self {
        if crate::zkey::on() {
            return String::from_utf8(crate::zkey::of_number(n, shrink)).expect("zkey keys are ASCII");
        }
        match (shrink, n) {
            (false, _) | (true, 32..) => format!("tenant/7/key/{n:07}"),
            (true, 0..8) => format!("t{n}"),
            (true, 8..16) => format!("tenant/{n:02}"),
            (true, 16..24) => format!("tenant/7/a{n:02}"),
            (true, _) => format!("tenant/7/key/!{n:02}"),
        }
    }
    fn number(&self, shrink: bool) -> u32 {
        if crate::zkey::on() {
            return crate::zkey::number_of(self.as_bytes(), shrink);
        }
        let digits = self.len() - self.bytes().rev().take_while(u8::is_ascii_digit).count();
        self[digits..].parse().unwrap_or(u32::MAX)
    }
    fn outside(n: u32) -> bool {
        if crate::zkey::on() {
            return crate::zkey::outside(n);
        }
        n < 32
    }
}

/// A value naming its key and the write: key number high, writer and
/// sequence low.
fn value(n: u32, thread: usize, seq: usize) -> u64 {
    (n as u64) << 32 | (thread as u64) << 24 | seq as u64 & 0xFF_FFFF
}

fn check_entry<K: Key>(k: &K, v: u64, shrink: bool) -> Result<(), String> {
    if (v >> 32) as u32 != k.number(shrink) {
        return Err(format!("key {k:?} holds a value of key number {}", v >> 32));
    }
    Ok(())
}

type Left = BTreeMap<u32, Option<u64>>;

/// The first stable key number in `lo..=hi` missing from `seen` (ascending).
fn missed(stable: &[bool], lo: u32, hi: u32, seen: &[u32]) -> Option<u32> {
    let mut seen = seen.iter().peekable();
    (lo..=hi).filter(|&n| stable[n as usize]).find(|&n| {
        while seen.next_if(|&&s| s < n).is_some() {}
        seen.next_if_eq(&&n).is_none()
    })
}

pub fn run<K: Key, S: InlineKeyStrategy<K> + Default + Send + Sync, const SHRINK: bool>(case: &Case) -> Result<(), Fail>
where
    BBTreeMap<K, u64, S>: Sync,
{
    let (space, preload, plans) = case;
    let space = *space;
    #[allow(unused_mut)]
    let mut map = BBTreeMap::<K, u64, S>::with_strategy(S::default());
    let m = |n: u32| n % space;
    for n in (0..(*preload).min(space)).filter(|&n| !(SHRINK && K::outside(n))) {
        map.insert(K::of(n, SHRINK), value(n, 7, 0)).map_err(|_| fail("syncthreads__refused__bulk_insert", format!("preload {n} refused")))?;
    }
    let mut writers: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    for (t, plan) in plans.iter().enumerate() {
        for op in plan {
            let keys: Vec<u32> = match *op {
                Op::Insert(k) | Op::Remove(k) | Op::Update(k) => vec![m(k)],
                Op::InsertMany(ks) => ks.iter().map(|&k| m(k)).collect(),
                Op::RemoveRun(a, n) => (0..n as u32).map(|i| m(a + i)).collect(),
                _ => vec![],
            };
            for k in keys {
                let w = writers.entry(k).or_default();
                if !w.contains(&t) {
                    w.push(t);
                }
            }
        }
    }
    // Preloaded keys no thread writes are present for the whole run: every
    // get finds them, and every scan and range_cloned over them visits them
    // ("an entry present for the whole scan is visited exactly once").
    let stable: Vec<bool> = (0..space)
        .map(|n| n < *preload && !(SHRINK && K::outside(n)) && !writers.contains_key(&n))
        .collect();
    let start = Barrier::new(plans.len());
    let results: Vec<Result<Left, Fail>> = std::thread::scope(|scope| {
        let handles: Vec<_> = plans
            .iter()
            .enumerate()
            .map(|(t, plan)| {
                let (map, start, stable) = (&map, &start, &stable);
                spawn_in(scope, move || -> Result<Left, Fail> {
                    let mut left = BTreeMap::new();
                    start.wait();
                    for (i, op) in plan.iter().enumerate() {
                        let ctx = |sig: &str| {
                            let sig = sig.to_string();
                            move |why: String| fail(&sig, format!("thread {t} op {i} {op:?}: {why}"))
                        };
                        match *op {
                            Op::Insert(k) => {
                                let v = value(m(k), t, i);
                                map.insert(K::of(m(k), SHRINK), v).map_err(|_| ctx("syncthreads__refused__insert")("refused".into()))?;
                                left.insert(m(k), Some(v));
                            }
                            Op::Remove(k) => {
                                if let Some(v) = map.remove(&K::of(m(k), SHRINK)) {
                                    check_entry(&K::of(m(k), SHRINK), v, SHRINK).map_err(ctx("syncthreads__torn__remove"))?;
                                }
                                left.insert(m(k), None);
                            }
                            Op::RemoveRun(a, n) => {
                                for j in 0..n as u32 {
                                    let k = m(a + j);
                                    if let Some(v) = map.remove(&K::of(k, SHRINK)) {
                                        check_entry(&K::of(k, SHRINK), v, SHRINK).map_err(ctx("syncthreads__torn__remove"))?;
                                    }
                                    left.insert(k, None);
                                }
                            }
                            Op::InsertMany(ks) => {
                                let batch = ks.map(|k| (K::of(m(k), SHRINK), value(m(k), t, i)));
                                for (r, k) in map.insert_many(batch).into_iter().zip(ks) {
                                    match r {
                                        Ok(Some(old)) => check_entry(&K::of(m(k), SHRINK), old, SHRINK).map_err(ctx("syncthreads__torn__insert_many"))?,
                                        Ok(None) => {}
                                        Err(_) => return Err(ctx("syncthreads__refused__insert_many")("refused".into())),
                                    }
                                    left.insert(m(k), Some(value(m(k), t, i)));
                                }
                            }
                            Op::Update(k) => {
                                let v = value(m(k), t, i);
                                let r = map.update_or_insert(K::of(m(k), SHRINK), v, |key, stored, new| {
                                    let ok = check_entry(key, *stored, SHRINK);
                                    *stored = new;
                                    ok
                                });
                                match r {
                                    Ok(Some(Err(e))) => return Err(ctx("syncthreads__torn__update")(e)),
                                    Ok(_) => {}
                                    Err(_) => return Err(ctx("syncthreads__refused__update")("refused".into())),
                                }
                                left.insert(m(k), Some(v));
                            }
                            Op::Get(k) => {
                                match map.get_copied(&K::of(m(k), SHRINK)) {
                                    Some(v) => check_entry(&K::of(m(k), SHRINK), v, SHRINK).map_err(ctx("syncthreads__torn__get"))?,
                                    None if stable[m(k) as usize] => return Err(ctx("syncthreads__missing__get")(format!("key number {} (preloaded, never written) not found", m(k)))),
                                    None => {}
                                }
                            }
                            Op::GetMany(ks) => {
                                let keys = ks.map(|k| K::of(m(k), SHRINK));
                                let got = map.get_many_copied([&keys[0], &keys[1], &keys[2], &keys[3]]);
                                for ((v, k), n) in got.into_iter().zip(&keys).zip(ks) {
                                    match v {
                                        Some(v) => check_entry(k, v, SHRINK).map_err(ctx("syncthreads__torn__get_many"))?,
                                        None if stable[m(n) as usize] => {
                                            return Err(ctx("syncthreads__missing__get_many")(format!("key number {} (preloaded, never written) not found in {ks:?}", m(n))));
                                        }
                                        None => {}
                                    }
                                }
                            }
                            Op::Scan(a, b, rev) => {
                                let (lo, hi) = (K::of(m(a).min(m(b)), SHRINK), K::of(m(a).max(m(b)), SHRINK));
                                let mut prev: Option<K> = None;
                                let mut bad: Option<(&str, String)> = None;
                                let mut seen = Vec::new();
                                let visit = |k: &K, v: &u64| {
                                    let ordered = prev.as_ref().is_none_or(|p| if rev { k < p } else { k > p });
                                    if !ordered || *k < lo || *k > hi {
                                        bad = Some(("syncthreads__order__scan", format!("key {k:?} after {prev:?}, range {lo:?}..={hi:?}")));
                                        return ControlFlow::Break(());
                                    }
                                    if let Err(e) = check_entry(k, *v, SHRINK) {
                                        bad = Some(("syncthreads__torn__scan", e));
                                        return ControlFlow::Break(());
                                    }
                                    prev = Some(k.clone());
                                    seen.push(k.number(SHRINK));
                                    ControlFlow::Continue(())
                                };
                                let range = (Bound::Included(lo.clone()), Bound::Included(hi.clone()));
                                if rev { map.scan_rev(range, visit) } else { map.scan(range, visit) };
                                if let Some((sig, why)) = bad {
                                    return Err(ctx(sig)(why));
                                }
                                if rev {
                                    seen.reverse();
                                }
                                if let Some(n) = missed(stable, m(a).min(m(b)), m(a).max(m(b)), &seen) {
                                    return Err(ctx("syncthreads__missing__scan")(format!("key number {n} (preloaded, never written) not visited")));
                                }
                            }
                            Op::Range(a, b) => {
                                let (lo, hi) = (K::of(m(a).min(m(b)), SHRINK), K::of(m(a).max(m(b)), SHRINK));
                                let mut prev: Option<K> = None;
                                let mut seen = Vec::new();
                                for (k, v) in map.range_cloned(lo.clone()..=hi.clone()) {
                                    if prev.as_ref().is_some_and(|p| &k <= p) || k < lo || k > hi {
                                        return Err(ctx("syncthreads__order__range")(format!("range_cloned handed {k:?} after {prev:?}")));
                                    }
                                    check_entry(&k, v, SHRINK).map_err(ctx("syncthreads__torn__range"))?;
                                    seen.push(k.number(SHRINK));
                                    prev = Some(k);
                                }
                                if let Some(n) = missed(stable, m(a).min(m(b)), m(a).max(m(b)), &seen) {
                                    return Err(ctx("syncthreads__missing__range")(format!("key number {n} (preloaded, never written) not handed out")));
                                }
                            }
                        }
                    }
                    Ok(left)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| {
                h.join().unwrap_or_else(|p| {
                    let msg = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()));
                    Err(fail("syncthreads__panic__thread", format!("thread panicked: {msg:?}")))
                })
            })
            .collect()
    });
    let mut lefts = Vec::new();
    for r in results {
        lefts.push(r?);
    }
    let mut bad = None;
    let mut count = 0usize;
    map.scan::<K, ()>(.., |k, v| {
        count += 1;
        match check_entry(k, *v, SHRINK) {
            Ok(()) => ControlFlow::Continue(()),
            Err(e) => {
                bad = Some(e);
                ControlFlow::Break(())
            }
        }
    });
    if let Some(e) = bad {
        return Err(fail("syncthreads__torn__final", format!("after the join: {e}")));
    }
    for n in (0..space).filter(|&n| stable[n as usize]) {
        let got = map.get_copied(&K::of(n, SHRINK));
        ensure(got == Some(value(n, 7, 0)), "syncthreads__missing__final", || format!("key number {n} (preloaded, never written) after the join: {got:?}"))?;
    }
    for (k, ws) in &writers {
        let [t] = ws[..] else { continue };
        let want = lefts[t].get(k).copied().flatten();
        let got = map.get_copied(&K::of(*k, SHRINK));
        ensure(got == want, "syncthreads__lost__single_writer", || format!("key {k}, written by thread {t} alone: {got:?}, it left {want:?}"))?;
    }
    #[cfg(fuzzing)]
    {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| map.check_invariants(count)));
        if let Err(p) = r {
            let msg = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()));
            return Err(fail("syncthreads__invariant__check", format!("after the join ({count} entries): {msg:?}")));
        }
    }
    let _ = count;
    Ok(())
}

#[derive(Default)]
pub struct Narrow;
impl bbtree::NodeConfig for Narrow {
    const INTERNAL_PAGE_SIZE: usize = 64;
    const PLAIN_INTERNAL_PAGE_SIZE: usize = 64;
}
