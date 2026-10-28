// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! `bbtree::BBTreeMap` (the map threads share) under an allocator that
//! refuses pages at chosen moments: a refused insert, `update_or_insert`,
//! `get_or_insert` or batch entry leaves the map unchanged, runs no
//! closure and hands its entry back; everything else answers as std's
//! `BTreeMap` does. Values count themselves, so that a refused entry
//! dropped twice, or never, shows; dropping the map gives back every page.

use std::alloc::Layout;
use std::collections::BTreeMap;
use std::fmt;
use std::ops::{Bound, ControlFlow};
use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};

use bbtree::{AllocError, Allocator, Auto, BBTreeMap, Global};
use ephyra::{BoxGen, GenExt, classify, gens};
use serde::{Deserialize, Serialize};

use crate::common::{Fail, ensure, fail, range_panics};

#[derive(Clone)]
pub struct Budget(Arc<(AtomicUsize, AtomicUsize)>);

impl Budget {
    pub(crate) fn new() -> Self {
        Budget(Arc::new((AtomicUsize::new(0), AtomicUsize::new(usize::MAX))))
    }
    pub(crate) fn set(&self, left: usize) {
        self.0.1.store(left, Ordering::Relaxed);
    }
    pub(crate) fn out(&self) -> usize {
        self.0.0.load(Ordering::Relaxed)
    }
}

unsafe impl Allocator for Budget {
    fn allocate_zeroed(&self, layout: Layout) -> Result<NonNull<u8>, AllocError> {
        let left = self.0.1.load(Ordering::Relaxed);
        if left == 0 {
            return Err(AllocError);
        }
        if left != usize::MAX {
            self.0.1.store(left - 1, Ordering::Relaxed);
        }
        self.0.0.fetch_add(1, Ordering::Relaxed);
        Global.allocate_zeroed(layout)
    }
    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        self.0.0.fetch_sub(1, Ordering::Relaxed);
        unsafe {
            ptr.write_bytes(0xA5, layout.size());
            Global.deallocate(ptr, layout);
        }
    }
}

/// A value that counts how many of it are alive.
pub struct Tracked {
    n: u64,
    live: Arc<AtomicIsize>,
}

impl Tracked {
    fn new(n: u64, live: &Arc<AtomicIsize>) -> Self {
        live.fetch_add(1, Ordering::Relaxed);
        Tracked { n, live: live.clone() }
    }
}

impl Clone for Tracked {
    fn clone(&self) -> Self {
        Tracked::new(self.n, &self.live)
    }
}

impl Drop for Tracked {
    fn drop(&mut self) {
        self.live.fetch_sub(1, Ordering::Relaxed);
    }
}

impl fmt::Debug for Tracked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.n)
    }
}

impl PartialEq for Tracked {
    fn eq(&self, other: &Self) -> bool {
        self.n == other.n
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Op {
    Insert(u64, u64),
    InsertMany([(u64, u64); 4]),
    /// Adds to the value, or inserts.
    UpdateOrInsert(u64, u64),
    GetOrInsert(u64, u64),
    Remove(u64),
    RemoveIfEven(u64),
    Scan(u64, u64),
    Budget(usize),
    Unlimited,
}

fn op(keys: BoxGen<u64>) -> BoxGen<Op> {
    let v = gens::int(0u64..=1000);
    gens::frequency(vec![
        (8, (keys.clone(), v.clone()).map(|(k, v)| Op::Insert(k, v)).boxed()),
        (
            2,
            (keys.clone(), keys.clone(), keys.clone(), keys.clone(), v.clone())
                .map(|(a, b, c, d, v)| Op::InsertMany([(a, v), (b, v + 1), (c, v + 2), (d, v + 3)]))
                .boxed(),
        ),
        (2, (keys.clone(), v.clone()).map(|(k, v)| Op::UpdateOrInsert(k, v)).boxed()),
        (2, (keys.clone(), v).map(|(k, v)| Op::GetOrInsert(k, v)).boxed()),
        (4, keys.clone().map(Op::Remove).boxed()),
        (1, keys.clone().map(Op::RemoveIfEven).boxed()),
        (1, (keys.clone(), keys).map(|(a, b)| Op::Scan(a, b)).boxed()),
        (2, gens::int(0usize..=6).map(Op::Budget).boxed()),
        (1, gens::constant(Op::Unlimited).boxed()),
    ])
    .boxed()
}

/// Keys dense or spread, so that shared prefixes grow and shrink.
fn keys() -> BoxGen<u64> {
    gens::frequency(vec![
        (4, gens::int(0u64..=300).boxed()),
        (2, gens::int(0u64..=20_000).map(|k| (0xAB << 40) + k * 7).boxed()),
        (1, gens::edges(0u64..=u64::MAX).boxed()),
    ])
    .boxed()
}

/// Entries bulk-loaded first, then ops.
pub type Case = (u16, Vec<Op>);

pub fn case() -> BoxGen<Case> {
    (gens::int(0u16..=3000), gens::vec(op(keys())).len(0..=300)).boxed()
}

/// A key type for the map: u64 itself (inline keys), or a String of it
/// (keys that own heap memory and compare whole).
pub trait Key: Ord + Clone + fmt::Debug + Send + Sync + 'static {
    fn of(n: u64) -> Self;
}
impl Key for u64 {
    fn of(n: u64) -> Self {
        n
    }
}
impl Key for String {
    /// Zero-padded, so that the strings sort as the numbers do.
    fn of(n: u64) -> Self {
        format!("{n:020}")
    }
}

pub fn run<K: Key>(case: &Case) -> Result<(), Fail>
where
    Auto: bbtree::InlineKeyStrategy<K>,
{
    let (bulk, ops) = case;
    let budget = Budget::new();
    let live = Arc::new(AtomicIsize::new(0));
    {
        let map = BBTreeMap::<K, Tracked, Auto, Budget>::new_in(budget.clone());
        let mut model: BTreeMap<K, u64> = BTreeMap::new();
        for n in 0..*bulk as u64 {
            let k = K::of(n * 13 % 4000);
            map.insert(k.clone(), Tracked::new(n, &live)).map_err(|_| fail("syncmap__error__bulk_insert", format!("bulk insert {n} refused")))?;
            model.insert(k, n);
        }
        for (step, op) in ops.iter().enumerate() {
            let at = |why: String| format!("op {step} {op:?}: {why}");
            // After a refusal the key holds what it held.
            let unchanged = |sig: &str, k: &K, model: &BTreeMap<K, u64>| -> Result<(), Fail> {
                classify(true, "refused");
                let got = map.get_cloned(k).map(|t| t.n);
                ensure(got == model.get(k).copied(), sig, || at(format!("after a refusal {k:?} holds {got:?}, before it {:?}", model.get(k))))
            };
            match op {
                Op::Insert(k, v) => {
                    let k = K::of(*k);
                    match map.insert(k.clone(), Tracked::new(*v, &live)) {
                        Ok(old) => {
                            let want = model.insert(k, *v);
                            ensure(old.as_ref().map(|t| t.n) == want, "syncmap__mismatch__insert", || {
                                at(format!("insert returned {old:?}, std {want:?}"))
                            })?;
                        }
                        Err(e) => {
                            let (rk, rv) = e.into_entry();
                            ensure(rk == k && rv.n == *v, "syncmap__refused_entry__insert", || {
                                at(format!("refused entry came back as ({rk:?}, {rv:?})"))
                            })?;
                            unchanged("syncmap__refused_changed__insert", &k, &model)?;
                        }
                    }
                }
                Op::InsertMany(entries) => {
                    let batch = entries.map(|(k, v)| (K::of(k), Tracked::new(v, &live)));
                    let results = map.insert_many(batch);
                    for (i, (result, (k, v))) in results.into_iter().zip(entries).enumerate() {
                        let k = K::of(*k);
                        match result {
                            Ok(old) => {
                                let want = model.insert(k, *v);
                                ensure(old.as_ref().map(|t| t.n) == want, "syncmap__mismatch__insert_many", || {
                                    at(format!("entry {i}: returned {old:?}, std {want:?}"))
                                })?;
                            }
                            Err(e) => {
                                classify(true, "refused_in_batch");
                                let (rk, rv) = e.into_entry();
                                ensure(rk == k && rv.n == *v, "syncmap__refused_entry__insert_many", || {
                                    at(format!("entry {i}: refused entry came back as ({rk:?}, {rv:?})"))
                                })?;
                            }
                        }
                    }
                    // The refused entries left the map as it was for them,
                    // which the model, updated by the accepted ones, is.
                    for (k, _) in entries {
                        let k = K::of(*k);
                        let got = map.get_cloned(&k).map(|t| t.n);
                        ensure(got == model.get(&k).copied(), "syncmap__refused_changed__insert_many", || {
                            at(format!("{k:?} holds {got:?}, std {:?}", model.get(&k)))
                        })?;
                    }
                }
                Op::UpdateOrInsert(k, v) => {
                    let k = K::of(*k);
                    let mut ran = false;
                    match map.update_or_insert(k.clone(), Tracked::new(*v, &live), |_, stored, add| {
                        ran = true;
                        stored.n += add.n;
                        stored.n
                    }) {
                        Ok(r) => {
                            let want = match model.get_mut(&k) {
                                Some(stored) => {
                                    *stored += v;
                                    Some(*stored)
                                }
                                None => {
                                    model.insert(k, *v);
                                    None
                                }
                            };
                            ensure(r == want, "syncmap__mismatch__update_or_insert", || at(format!("returned {r:?}, std {want:?}")))?;
                        }
                        Err(e) => {
                            let (rk, rv) = e.into_entry();
                            ensure(!ran, "syncmap__closure_ran__update_or_insert", || at("the closure ran on a refused insert".into()))?;
                            ensure(rk == k && rv.n == *v, "syncmap__refused_entry__update_or_insert", || {
                                at(format!("refused entry came back as ({rk:?}, {rv:?})"))
                            })?;
                            unchanged("syncmap__refused_changed__update_or_insert", &k, &model)?;
                        }
                    }
                }
                Op::GetOrInsert(k, v) => {
                    let k = K::of(*k);
                    let mut ran = false;
                    match map.get_or_insert(k.clone(), Tracked::new(*v, &live), |_, stored| {
                        ran = true;
                        stored.n
                    }) {
                        Ok(seen) => {
                            let want = *model.entry(k).or_insert(*v);
                            ensure(seen == want && ran, "syncmap__mismatch__get_or_insert", || at(format!("saw {seen} (ran {ran}), std {want}")))?;
                        }
                        Err(e) => {
                            let (rk, rv) = e.into_entry();
                            ensure(!ran, "syncmap__closure_ran__get_or_insert", || at("the closure ran on a refused insert".into()))?;
                            ensure(rk == k && rv.n == *v, "syncmap__refused_entry__get_or_insert", || {
                                at(format!("refused entry came back as ({rk:?}, {rv:?})"))
                            })?;
                            unchanged("syncmap__refused_changed__get_or_insert", &k, &model)?;
                        }
                    }
                }
                Op::Remove(k) => {
                    let k = K::of(*k);
                    let (got, want) = (map.remove(&k).map(|t| t.n), model.remove(&k));
                    ensure(got == want, "syncmap__mismatch__remove", || at(format!("remove {got:?}, std {want:?}")))?;
                }
                Op::RemoveIfEven(k) => {
                    let k = K::of(*k);
                    let got = map.remove_if(&k, |_, v| v.n % 2 == 0).map(|(_, v)| v.n);
                    let want = match model.get(&k) {
                        Some(v) if v % 2 == 0 => model.remove(&k),
                        _ => None,
                    };
                    ensure(got == want, "syncmap__mismatch__remove_if", || at(format!("remove_if {got:?}, std {want:?}")))?;
                }
                Op::Scan(a, b) => {
                    let (s, e) = (Bound::Included(K::of(*a)), Bound::Excluded(K::of(*b)));
                    if !range_panics(&s, &e) {
                        let mut got = Vec::new();
                        map.scan((s.clone(), e.clone()), |k, v| {
                            got.push((k.clone(), v.n));
                            ControlFlow::<()>::Continue(())
                        });
                        let want: Vec<_> = model.range((s, e)).map(|(k, v)| (k.clone(), *v)).collect();
                        ensure(got == want, "syncmap__mismatch__scan", || at(format!("scan {} entries, std {}", got.len(), want.len())))?;
                    }
                }
                Op::Budget(n) => budget.set(*n),
                Op::Unlimited => budget.set(usize::MAX),
            }
            let alive = live.load(Ordering::Relaxed);
            ensure(alive == model.len() as isize, "syncmap__drop_count__values", || {
                at(format!("{alive} values alive, the map holds {}", model.len()))
            })?;
        }
        budget.set(usize::MAX);
        let all: Vec<_> = map.range_cloned::<K, _>(..).map(|(k, v)| (k, v.n)).collect();
        let want: Vec<_> = model.into_iter().collect();
        ensure(all == want, "syncmap__mismatch__end", || format!("at the end: {} entries, std {}", all.len(), want.len()))?;
    }
    let (out, alive) = (budget.out(), live.load(Ordering::Relaxed));
    ensure(out == 0 && alive == 0, "syncmap__leak__drop", || format!("after the drop: {out} pages out, {alive} values alive"))
}
