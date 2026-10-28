// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! `bbtree-store` shared by threads: writers insert, overwrite and remove
//! overlapping keys, with values of every length (overflow pages
//! included), while readers get, scan and range. Every value describes
//! itself (its key, a version, a length, a fill derived from them), so a
//! reader that sees a torn record, another key's record, or a freed page
//! (which the allocator scribbles over) sees a value that does not check.
//! Scans must hand out keys in order, within the range, each once. After
//! the threads join, a key that one thread alone wrote holds what that
//! thread left. Nondeterministic: repro tries several times.

use std::alloc::Layout;
use std::collections::BTreeMap;
use std::ops::{Bound, ControlFlow};
use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use bbtree_store::BBTreeStore;
use bbtree_store::bbtree::{AllocError, Allocator, Global, NodeConfig};
use ephyra::{BoxGen, GenExt, gens};
use serde::{Deserialize, Serialize};

use crate::common::{Fail, ensure, fail, spawn_in};

/// Counts pages out and scribbles over pages given back.
#[derive(Clone, Default)]
pub struct Scribbling(Arc<AtomicUsize>);

unsafe impl Allocator for Scribbling {
    fn allocate_zeroed(&self, layout: Layout) -> Result<NonNull<u8>, AllocError> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Global.allocate_zeroed(layout)
    }
    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        self.0.fetch_sub(1, Ordering::Relaxed);
        unsafe {
            ptr.write_bytes(0xA5, layout.size());
            Global.deallocate(ptr, layout);
        }
    }
}

#[derive(Default)]
pub struct SmallInternals;
impl NodeConfig for SmallInternals {
    const INTERNAL_PAGE_SIZE: usize = 64;
    const PLAIN_INTERNAL_PAGE_SIZE: usize = 64;
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum ThreadOp {
    /// Key number, value length.
    Insert(u16, u32),
    Remove(u16),
    Get(u16),
    Scan { from: u16, to: u16, rev: bool },
    Range { from: u16, to: u16, back_every: u8 },
}

/// Key `n`: lengths around the inline key, shared prefixes.
fn key(n: u16) -> Vec<u8> {
    let mut k = vec![b'k', (n >> 8) as u8, n as u8];
    k.extend(std::iter::repeat_n(b'x', (n as usize * 7) % 23));
    k
}

/// The value of key `n` at `version`, `len` bytes long (at least its
/// 10-byte header): key number, version, length, then a fill.
fn value(n: u16, version: u32, len: u32) -> Vec<u8> {
    let len = len.max(10) as usize;
    let mut v = Vec::with_capacity(len);
    v.extend_from_slice(&n.to_le_bytes());
    v.extend_from_slice(&version.to_le_bytes());
    v.extend_from_slice(&(len as u32).to_le_bytes());
    let seed = (n as u32).wrapping_mul(2654435761) ^ version;
    v.extend((10..len).map(|i| (seed.wrapping_add(i as u32).wrapping_mul(0x9E37) >> 7) as u8));
    v
}

/// Whether `v` is a value of key `k` that `value` made; why not, if not.
fn check_value(k: &[u8], v: &[u8]) -> Result<(), String> {
    if v.len() < 10 {
        return Err(format!("value of {} bytes, shorter than its header", v.len()));
    }
    let n = u16::from_le_bytes([v[0], v[1]]);
    let version = u32::from_le_bytes([v[2], v[3], v[4], v[5]]);
    let len = u32::from_le_bytes([v[6], v[7], v[8], v[9]]);
    if key(n) != k {
        return Err(format!("key {k:?} holds a value of key {:?}", key(n)));
    }
    if len as usize != v.len() || value(n, version, len) != v {
        let at = value(n, version, len).iter().zip(v).position(|(a, b)| a != b);
        return Err(format!("value of key {n} version {version}: {} bytes, header says {len}, first wrong byte at {at:?}", v.len()));
    }
    Ok(())
}

fn thread_op(keys: u16) -> BoxGen<ThreadOp> {
    let k = gens::int(0u16..keys);
    let len = gens::frequency(vec![
        (5, gens::int(0u32..=40).boxed()),
        (3, gens::int(0u32..=600).boxed()),
        (2, gens::int(900u32..=1200).boxed()),
        (1, gens::int(1200u32..=9000).boxed()),
    ]);
    gens::frequency(vec![
        (6, (k.clone(), len).map(|(k, l)| ThreadOp::Insert(k, l)).boxed()),
        (3, k.clone().map(ThreadOp::Remove).boxed()),
        (2, k.clone().map(ThreadOp::Get).boxed()),
        (1, (k.clone(), k.clone(), gens::bool()).map(|(from, to, rev)| ThreadOp::Scan { from, to, rev }).boxed()),
        (1, (k.clone(), k, gens::int(0u8..=4)).map(|(from, to, back_every)| ThreadOp::Range { from, to, back_every }).boxed()),
    ])
    .boxed()
}

/// Key space size (key numbers are taken modulo it), bulk-loaded keys,
/// one plan per thread.
pub type Case = (u16, u16, Vec<Vec<ThreadOp>>);

pub fn case() -> BoxGen<Case> {
    (
        gens::frequency(vec![(2, gens::int(4u16..=40).boxed()), (2, gens::int(4u16..=600).boxed())]),
        gens::int(0u16..=600),
        gens::vec(gens::vec(thread_op(600)).len(0..=400)).len(2..=4),
    )
        .boxed()
}

type Left = BTreeMap<u16, Option<Vec<u8>>>;

pub fn run<C: NodeConfig + Default + Send + Sync + 'static>(case: &Case) -> Result<(), Fail>
where
    BBTreeStore<Scribbling, C>: Sync,
{
    let (keys, bulk, plans) = case;
    let keys = *keys;
    let alloc = Scribbling::default();
    {
        let store = BBTreeStore::with_config_in(C::default(), alloc.clone());
        let bulk = (*bulk).min(keys);
        let plans: Vec<Vec<ThreadOp>> = plans
            .iter()
            .map(|plan| {
                plan.iter()
                    .map(|op| match *op {
                        ThreadOp::Insert(k, l) => ThreadOp::Insert(k % keys, l),
                        ThreadOp::Remove(k) => ThreadOp::Remove(k % keys),
                        ThreadOp::Get(k) => ThreadOp::Get(k % keys),
                        ThreadOp::Scan { from, to, rev } => ThreadOp::Scan { from: from % keys, to: to % keys, rev },
                        ThreadOp::Range { from, to, back_every } => ThreadOp::Range { from: from % keys, to: to % keys, back_every },
                    })
                    .collect()
            })
            .collect();
        for n in 0..bulk {
            store
                .insert(&key(n), &value(n, 0, n as u32 * 13 % 1500))
                .map_err(|e| fail("storethreads__error__bulk_insert", format!("bulk insert {n}: {e}")))?;
        }
        // Which threads write each key, and what each left there.
        let mut writers: BTreeMap<u16, Vec<usize>> = BTreeMap::new();
        for (t, plan) in plans.iter().enumerate() {
            for op in plan {
                if let ThreadOp::Insert(k, _) | ThreadOp::Remove(k) = op {
                    let w = writers.entry(*k).or_default();
                    if !w.contains(&t) {
                        w.push(t);
                    }
                }
            }
        }
        // Bulk-loaded keys no thread writes are present for the whole run:
        // gets find them, scans and ranges over them hand them out (a scan
        // "as a map's scan": "an entry present for the whole scan is visited
        // exactly once").
        let stable: Vec<bool> = (0..keys).map(|n| n < bulk && !writers.contains_key(&n)).collect();
        let missed = |lo: u16, hi: u16, seen: &std::collections::BTreeSet<Vec<u8>>| (lo..=hi).find(|&n| stable[n as usize] && !seen.contains(&key(n)));
        let start = std::sync::Barrier::new(plans.len());
        let outcomes: Vec<Result<Left, Fail>> = std::thread::scope(|scope| {
            let handles: Vec<_> = plans
                .iter()
                .enumerate()
                .map(|(t, plan)| {
                    let (store, start, stable, missed) = (&store, &start, &stable, &missed);
                    spawn_in(scope, move || -> Result<Left, Fail> {
                        let mut left = BTreeMap::new();
                        start.wait();
                        for (i, op) in plan.iter().enumerate() {
                            let ctx = |sig: &str| {
                                let sig = sig.to_string();
                                move |why: String| fail(&sig, format!("thread {t} op {i} {op:?}: {why}"))
                            };
                            match *op {
                                ThreadOp::Insert(k, len) => {
                                    let v = value(k, (t as u32) << 16 | i as u32, len);
                                    store.insert(&key(k), &v).map_err(|e| ctx("storethreads__error__insert")(format!("{e}")))?;
                                    left.insert(k, Some(v));
                                }
                                ThreadOp::Remove(k) => {
                                    store.remove(&key(k));
                                    left.insert(k, None);
                                }
                                ThreadOp::Get(k) => {
                                    match store.get(&key(k)) {
                                        Some(v) => check_value(&key(k), &v).map_err(ctx("storethreads__torn__get"))?,
                                        None if stable[k as usize] => return Err(ctx("storethreads__missing__get")(format!("key {k} (bulk-loaded, never written) not found"))),
                                        None => {}
                                    }
                                }
                                ThreadOp::Scan { from, to, rev } => {
                                    let (lo, hi) = (key(from.min(to)), key(from.max(to)));
                                    let mut prev: Option<Vec<u8>> = None;
                                    let mut bad: Option<(&str, String)> = None;
                                    let mut seen = std::collections::BTreeSet::new();
                                    let visit = |k: &[u8], v: &[u8]| {
                                        let ordered = prev.as_deref().is_none_or(|p| if rev { k < p } else { k > p });
                                        if !ordered || k < &lo[..] || k > &hi[..] {
                                            bad = Some(("storethreads__order__scan", format!("key {k:?} after {prev:?}, range {lo:?}..={hi:?}")));
                                            return ControlFlow::Break(());
                                        }
                                        if let Err(e) = check_value(k, v) {
                                            bad = Some(("storethreads__torn__scan", e));
                                            return ControlFlow::Break(());
                                        }
                                        prev = Some(k.to_vec());
                                        seen.insert(k.to_vec());
                                        ControlFlow::Continue(())
                                    };
                                    let range = (Bound::Included(&lo[..]), Bound::Included(&hi[..]));
                                    if rev { store.scan_rev::<[u8], ()>(range, visit) } else { store.scan::<[u8], ()>(range, visit) };
                                    if let Some((sig, why)) = bad {
                                        return Err(ctx(sig)(why));
                                    }
                                    if let Some(n) = missed(from.min(to), from.max(to), &seen) {
                                        return Err(ctx("storethreads__missing__scan")(format!("key {n} (bulk-loaded, never written) not visited")));
                                    }
                                }
                                ThreadOp::Range { from, to, back_every } => {
                                    let (lo, hi) = (key(from.min(to)), key(from.max(to)));
                                    let mut it = store.range::<[u8], _>((Bound::Included(&lo[..]), Bound::Included(&hi[..])));
                                    let (mut front, mut back): (Option<Vec<u8>>, Option<Vec<u8>>) = (None, None);
                                    let mut step = 0u32;
                                    let mut seen = std::collections::BTreeSet::new();
                                    loop {
                                        step += 1;
                                        let from_back = back_every != 0 && step % u32::from(back_every) == 0;
                                        let Some((k, v)) = (if from_back { it.next_back() } else { it.next() }) else { break };
                                        check_value(&k, &v).map_err(ctx("storethreads__torn__range"))?;
                                        let in_range = k >= lo && k <= hi;
                                        let after_front = front.as_ref().is_none_or(|f| &k > f);
                                        let before_back = back.as_ref().is_none_or(|b| &k < b);
                                        if !in_range || !after_front || !before_back {
                                            return Err(ctx("storethreads__order__range")(format!(
                                                "range handed {k:?} ({}), front at {front:?}, back at {back:?}",
                                                if from_back { "next_back" } else { "next" }
                                            )));
                                        }
                                        seen.insert(k.clone());
                                        if from_back {
                                            back = Some(k)
                                        } else {
                                            front = Some(k)
                                        }
                                        if step > 100_000 {
                                            return Err(ctx("storethreads__runaway__range")("range handed out more than 100000 entries".into()));
                                        }
                                    }
                                    if let Some(n) = missed(from.min(to), from.max(to), &seen) {
                                        return Err(ctx("storethreads__missing__range")(format!("key {n} (bulk-loaded, never written) not handed out")));
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
                        Err(fail("storethreads__panic__thread", format!("thread panicked: {msg:?}")))
                    })
                })
                .collect()
        });
        let mut lefts = Vec::new();
        for outcome in outcomes {
            lefts.push(outcome?);
        }
        // Every entry checks; keys one thread alone wrote hold its last write.
        let mut bad = None;
        store.scan::<[u8], ()>(.., |k, v| match check_value(k, v) {
            Ok(()) => ControlFlow::Continue(()),
            Err(e) => {
                bad = Some(e);
                ControlFlow::Break(())
            }
        });
        if let Some(e) = bad {
            return Err(fail("storethreads__torn__final", format!("after the threads: {e}")));
        }
        for n in (0..keys).filter(|&n| stable[n as usize]) {
            let got = store.get(&key(n));
            ensure(got.as_deref() == Some(&value(n, 0, n as u32 * 13 % 1500)[..]), "storethreads__missing__final", || {
                format!("key {n} (bulk-loaded, never written) after the threads: {:?} bytes", got.as_ref().map(|v| v.len()))
            })?;
        }
        for n in 0..keys {
            let Some(ws) = writers.get(&n) else { continue };
            let [t] = ws[..] else { continue };
            let want = lefts[t].get(&n).cloned().flatten();
            let got = store.get(&key(n));
            ensure(got == want, "storethreads__lost__single_writer", || {
                format!("key {n}, written by thread {t} alone: {:?} bytes, it left {:?}", got.as_ref().map(|v| v.len()), want.as_ref().map(|v| v.len()))
            })?;
        }
    }
    let out = alloc.0.load(Ordering::Relaxed);
    ensure(out == 0, "storethreads__leak__drop", || format!("{out} allocations still out after the store was dropped"))
}
