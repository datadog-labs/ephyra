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

use crate::common::{Fail, ensure, fail};

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
    let k = gens::int(0u32..space);
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
        gens::frequency(vec![(1, gens::int(64u32..=2000).boxed()), (2, gens::int(2000u32..=SPACE).boxed())]),
        gens::int(0u32..=SPACE),
        gens::vec(gens::vec(op(SPACE)).len(0..=3000)).len(2..=6),
    )
        .boxed()
}

pub trait Key: Ord + Clone + Debug + Send + Sync + 'static {
    fn of(n: u32) -> Self;
    fn number(&self) -> u32;
}
impl Key for u64 {
    fn of(n: u32) -> Self {
        (0xAB << 40) + n as u64 * 3
    }
    fn number(&self) -> u32 {
        ((self - (0xAB << 40)) / 3) as u32
    }
}
impl Key for String {
    fn of(n: u32) -> Self {
        format!("tenant/7/key/{n:07}")
    }
    fn number(&self) -> u32 {
        self[13..].parse().unwrap_or(u32::MAX)
    }
}

/// A value naming its key and the write: key number high, writer and
/// sequence low.
fn value(n: u32, thread: usize, seq: usize) -> u64 {
    (n as u64) << 32 | (thread as u64) << 24 | seq as u64 & 0xFF_FFFF
}

fn check_entry<K: Key>(k: &K, v: u64) -> Result<(), String> {
    if (v >> 32) as u32 != k.number() {
        return Err(format!("key {k:?} holds a value of key number {}", v >> 32));
    }
    Ok(())
}

type Left = BTreeMap<u32, Option<u64>>;

pub fn run<K: Key, S: InlineKeyStrategy<K> + Default + Send + Sync>(case: &Case) -> Result<(), Fail>
where
    BBTreeMap<K, u64, S>: Sync,
{
    let (space, preload, plans) = case;
    let space = *space;
    #[allow(unused_mut)]
    let mut map = BBTreeMap::<K, u64, S>::with_strategy(S::default());
    let m = |n: u32| n % space;
    for n in 0..(*preload).min(space) {
        map.insert(K::of(n), value(n, 7, 0)).map_err(|_| fail("syncthreads__refused__bulk_insert", format!("preload {n} refused")))?;
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
    let start = Barrier::new(plans.len());
    let results: Vec<Result<Left, Fail>> = std::thread::scope(|scope| {
        let handles: Vec<_> = plans
            .iter()
            .enumerate()
            .map(|(t, plan)| {
                let (map, start) = (&map, &start);
                scope.spawn(move || -> Result<Left, Fail> {
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
                                map.insert(K::of(m(k)), v).map_err(|_| ctx("syncthreads__refused__insert")("refused".into()))?;
                                left.insert(m(k), Some(v));
                            }
                            Op::Remove(k) => {
                                if let Some(v) = map.remove(&K::of(m(k))) {
                                    check_entry(&K::of(m(k)), v).map_err(ctx("syncthreads__torn__remove"))?;
                                }
                                left.insert(m(k), None);
                            }
                            Op::RemoveRun(a, n) => {
                                for j in 0..n as u32 {
                                    let k = m(a + j);
                                    if let Some(v) = map.remove(&K::of(k)) {
                                        check_entry(&K::of(k), v).map_err(ctx("syncthreads__torn__remove"))?;
                                    }
                                    left.insert(k, None);
                                }
                            }
                            Op::InsertMany(ks) => {
                                let batch = ks.map(|k| (K::of(m(k)), value(m(k), t, i)));
                                for (r, k) in map.insert_many(batch).into_iter().zip(ks) {
                                    match r {
                                        Ok(Some(old)) => check_entry(&K::of(m(k)), old).map_err(ctx("syncthreads__torn__insert_many"))?,
                                        Ok(None) => {}
                                        Err(_) => return Err(ctx("syncthreads__refused__insert_many")("refused".into())),
                                    }
                                    left.insert(m(k), Some(value(m(k), t, i)));
                                }
                            }
                            Op::Update(k) => {
                                let v = value(m(k), t, i);
                                let r = map.update_or_insert(K::of(m(k)), v, |key, stored, new| {
                                    let ok = check_entry(key, *stored);
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
                                if let Some(v) = map.get_copied(&K::of(m(k))) {
                                    check_entry(&K::of(m(k)), v).map_err(ctx("syncthreads__torn__get"))?;
                                }
                            }
                            Op::GetMany(ks) => {
                                let keys = ks.map(|k| K::of(m(k)));
                                let got = map.get_many_copied([&keys[0], &keys[1], &keys[2], &keys[3]]);
                                for (v, k) in got.into_iter().zip(&keys) {
                                    if let Some(v) = v {
                                        check_entry(k, v).map_err(ctx("syncthreads__torn__get_many"))?;
                                    }
                                }
                            }
                            Op::Scan(a, b, rev) => {
                                let (lo, hi) = (K::of(m(a).min(m(b))), K::of(m(a).max(m(b))));
                                let mut prev: Option<K> = None;
                                let mut bad: Option<(&str, String)> = None;
                                let visit = |k: &K, v: &u64| {
                                    let ordered = prev.as_ref().is_none_or(|p| if rev { k < p } else { k > p });
                                    if !ordered || *k < lo || *k > hi {
                                        bad = Some(("syncthreads__order__scan", format!("key {k:?} after {prev:?}, range {lo:?}..={hi:?}")));
                                        return ControlFlow::Break(());
                                    }
                                    if let Err(e) = check_entry(k, *v) {
                                        bad = Some(("syncthreads__torn__scan", e));
                                        return ControlFlow::Break(());
                                    }
                                    prev = Some(k.clone());
                                    ControlFlow::Continue(())
                                };
                                let range = (Bound::Included(lo.clone()), Bound::Included(hi.clone()));
                                if rev { map.scan_rev(range, visit) } else { map.scan(range, visit) };
                                if let Some((sig, why)) = bad {
                                    return Err(ctx(sig)(why));
                                }
                            }
                            Op::Range(a, b) => {
                                let (lo, hi) = (K::of(m(a).min(m(b))), K::of(m(a).max(m(b))));
                                let mut prev: Option<K> = None;
                                for (k, v) in map.range_cloned(lo.clone()..=hi.clone()) {
                                    if prev.as_ref().is_some_and(|p| &k <= p) || k < lo || k > hi {
                                        return Err(ctx("syncthreads__order__range")(format!("range_cloned handed {k:?} after {prev:?}")));
                                    }
                                    check_entry(&k, v).map_err(ctx("syncthreads__torn__range"))?;
                                    prev = Some(k);
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
        match check_entry(k, *v) {
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
    for (k, ws) in &writers {
        let [t] = ws[..] else { continue };
        let want = lefts[t].get(k).copied().flatten();
        let got = map.get_copied(&K::of(*k));
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
