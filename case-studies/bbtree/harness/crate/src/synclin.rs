// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! `bbtree::BBTreeMap` shared by 2 to 6 threads, checked while they run,
//! not only after the join. Each thread owns its own keys, interleaved with
//! every other thread's (key numbers `(i * threads + t) * 2 + 1`), so their
//! leaves are shared and split, merge and lower the root under all of them,
//! yet no other thread changes them: every read and write of a thread's own
//! keys has one right answer, from its own model, at every step (insert,
//! remove, remove_if, update, get_or_insert, update_or_insert, the batches
//! of 1, 4, 16 and 70 lanes, get_copied / get_cloned / read / contains_key,
//! get_many_*, read_many, scans and range_cloned from both ends, is_empty).
//! Shared hot keys (even numbers, never removed) count additions made by
//! `update_or_insert` and `update_or_insert_many` ("threads that add to the
//! same key's value this way miss none of each other's additions"): the
//! final count is the sum of every thread's additions, and a thread's reads
//! of a hot count never go back below what it saw or wrote before. Values
//! are 3 to 8 words that check each other, so an optimistic copy of a value
//! being written (`get_copied`: "checks that no thread changed its leaf
//! meanwhile") shows as torn.
//! Nondeterministic: repro tries several times.

use std::alloc::Layout;
use std::cell::Cell;
use std::collections::BTreeMap;
use std::fmt::Debug;
use std::ops::{Bound, ControlFlow};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Arc, Barrier};

use bbtree::{AllocError, Allocator, BBTreeMap, Global, InlineKeyStrategy};
use ephyra::{BoxGen, GenExt, classify, gens};
use serde::{Deserialize, Serialize};

use crate::common::{Fail, ensure, fail, spawn_in};
use crate::syncthreads::Key;

/// A value of `N` words: key number, payload, and `N - 2` words derived
/// from both, so a copy that mixes two writes fails [`W::check`].
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct W<const N: usize>([u64; N]);

fn mix(n: u64, p: u64, i: usize) -> u64 {
    let mut x = n.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ p.rotate_left(17) ^ (i as u64).wrapping_mul(0xD6E8_FEB8_6659_FD93);
    x ^= x >> 29;
    x.wrapping_mul(0xBF58_476D_1CE4_E5B9)
}

impl<const N: usize> W<N> {
    fn new(n: u32, payload: u64) -> Self {
        let mut w = [0u64; N];
        w[0] = n as u64;
        w[1] = payload;
        for (i, x) in w.iter_mut().enumerate().skip(2) {
            *x = mix(n as u64, payload, i);
        }
        W(w)
    }
    fn payload(&self) -> u64 {
        self.0[1]
    }
    /// The value is whole and names key number `n`.
    fn check(&self, n: u32) -> Result<(), String> {
        if self.0[0] != n as u64 {
            return Err(format!("key number {n} holds a value of key number {}: {self:?}", self.0[0]));
        }
        if (2..N).any(|i| self.0[i] != mix(self.0[0], self.0[1], i)) {
            return Err(format!("key number {n}: torn value {self:?}"));
        }
        Ok(())
    }
}

impl<const N: usize> Debug for W<N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "W(n={}, p={:#x}{})", self.0[0], self.0[1], if (2..N).all(|i| self.0[i] == mix(self.0[0], self.0[1], i)) { "" } else { ", TORN" })
    }
}

/// A thread's own write: thread in the high bits, op index low.
fn own_payload(t: usize, i: usize) -> u64 {
    (t as u64 + 1) << 40 | i as u64
}
const PRELOAD: u64 = 0xFF << 40;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Read {
    Copied,
    Cloned,
    Read,
    Contains,
}

/// A key a batch read picks: one of the thread's own, a hot one, or one of
/// another thread's (checked only for being whole and named right).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Pick {
    Own(u16),
    Hot(u8),
    Any(u32),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Op {
    Insert(u16),
    Remove(u16),
    RemoveRun(u16, u16),
    RemoveIf(u16, bool),
    Update(u16),
    GetOrInsert(u16),
    Upsert(u16),
    /// `insert_many` (false) or `update_or_insert_many` (true) of own keys
    /// (`Own`, replaced) and hot keys (`Hot`, added to by `d`).
    Batch(Vec<(Pick, u8)>, bool),
    Get(u16, Read),
    GetMany(Vec<Pick>, Read),
    Add(u8, u8),
    /// get_or_insert a hot key with count 0, then `update` it by `d`.
    AddByUpdate(u8, u8),
    Peek(u8, Read),
    /// Scan key numbers `a..=b` forward or in reverse.
    Scan(u32, u32, bool),
    /// range_cloned over `a..=b`, taking from the front (false) or back.
    Range(u32, u32, Vec<bool>),
    IsEmpty,
}

/// Threads' own keys per thread, hot keys, hot spacing (in own-key
/// strides), preload percent, value width variant, one plan per thread.
pub type Case = (u16, u8, u16, u8, Vec<Vec<Op>>);

fn op(own: u16) -> BoxGen<Op> {
    // Own indices: mostly anywhere, sometimes a handful at the start, so
    // that one leaf fills and empties under every thread.
    let i = gens::frequency(vec![(3, gens::int(0u16..own).boxed()), (1, gens::int(0u16..8).boxed())]).boxed();
    let h = gens::int(0u8..16).boxed();
    let d = gens::int(1u8..=255).boxed();
    let num = gens::int(0u32..(own as u32) * 12 + 64).boxed();
    let read = gens::frequency(vec![
        (3, gens::constant(Read::Copied).boxed()),
        (1, gens::constant(Read::Cloned).boxed()),
        (1, gens::constant(Read::Read).boxed()),
        (1, gens::constant(Read::Contains).boxed()),
    ])
    .boxed();
    let pick = gens::frequency(vec![
        (4, i.clone().map(Pick::Own).boxed()),
        (2, h.clone().map(Pick::Hot).boxed()),
        (1, num.clone().map(Pick::Any).boxed()),
    ])
    .boxed();
    let wpick = gens::frequency(vec![(3, i.clone().map(Pick::Own).boxed()), (1, h.clone().map(Pick::Hot).boxed())]).boxed();
    let batch_len = |g: BoxGen<(Pick, u8)>| {
        gens::frequency(vec![
            (1, gens::vec(g.clone()).len(1..=1).boxed()),
            (4, gens::vec(g.clone()).len(4..=4).boxed()),
            (2, gens::vec(g.clone()).len(16..=16).boxed()),
            (1, gens::vec(g).len(70..=70).boxed()),
        ])
        .boxed()
    };
    let picks = |g: BoxGen<Pick>| {
        gens::frequency(vec![
            (1, gens::vec(g.clone()).len(1..=1).boxed()),
            (4, gens::vec(g.clone()).len(4..=4).boxed()),
            (2, gens::vec(g.clone()).len(16..=16).boxed()),
            (1, gens::vec(g).len(70..=70).boxed()),
        ])
        .boxed()
    };
    gens::frequency(vec![
        (6, i.clone().map(Op::Insert).boxed()),
        (5, i.clone().map(Op::Remove).boxed()),
        (1, (i.clone(), gens::int(1u16..=600)).map(|(a, n)| Op::RemoveRun(a, n)).boxed()),
        (2, (i.clone(), gens::bool()).map(|(a, b)| Op::RemoveIf(a, b)).boxed()),
        (2, i.clone().map(Op::Update).boxed()),
        (2, i.clone().map(Op::GetOrInsert).boxed()),
        (2, i.clone().map(Op::Upsert).boxed()),
        (2, (batch_len((wpick, d.clone()).boxed()), gens::bool()).map(|(b, u)| Op::Batch(b, u)).boxed()),
        (5, (i.clone(), read.clone()).map(|(a, r)| Op::Get(a, r)).boxed()),
        (2, (picks(pick), read.clone()).map(|(p, r)| Op::GetMany(p, r)).boxed()),
        (4, (h.clone(), d.clone()).map(|(h, d)| Op::Add(h, d)).boxed()),
        (1, (h.clone(), d).map(|(h, d)| Op::AddByUpdate(h, d)).boxed()),
        (3, (h, read).map(|(h, r)| Op::Peek(h, r)).boxed()),
        (1, (num.clone(), num.clone(), gens::bool()).map(|(a, b, r)| Op::Scan(a.min(b), a.max(b), r)).boxed()),
        (1, (num.clone(), num, gens::vec(gens::bool()).len(0..=40)).map(|(a, b, p)| Op::Range(a.min(b), a.max(b), p)).boxed()),
        (1, gens::constant(Op::IsEmpty).boxed()),
    ])
    .boxed()
}

pub fn case() -> BoxGen<Case> {
    let own = gens::frequency(vec![(1, gens::int(1u16..=16).boxed()), (2, gens::int(16u16..=600).boxed()), (2, gens::int(600u16..=8000).boxed())]);
    gens::dependent(own, |&own| {
        (
            gens::int(1u8..=16),
            gens::frequency(vec![(1, gens::int(0u16..=4).boxed()), (2, gens::int(1u16..=own.max(1)).boxed())]),
            gens::frequency(vec![(1, gens::constant(0u8).boxed()), (2, gens::int(0u8..=100).boxed())]),
            gens::vec(gens::vec(op(own)).len(0..=1500)).len(2..=6),
        )
    })
    .map(|(own, (hots, spacing, preload, plans))| (own, hots, spacing, preload, plans))
    .boxed()
}

struct Shape {
    threads: u32,
    own: u32,
    hots: u32,
    spacing: u32,
}

impl Shape {
    fn own_number(&self, t: usize, i: u32) -> u32 {
        (i * self.threads + t as u32) * 2 + 1
    }
    /// Hot keys are even, spread `spacing` own strides apart.
    fn hot_number(&self, h: u32) -> u32 {
        h % self.hots * self.spacing.max(1) * self.threads * 2
    }
    fn hot_index(&self, n: u32) -> Option<u32> {
        let step = self.spacing.max(1) * self.threads * 2;
        (n % 2 == 0 && n % step == 0 && n / step < self.hots).then_some(n / step)
    }
    /// Owner and own index of an odd key number.
    fn owner(&self, n: u32) -> Option<(usize, u32)> {
        (n % 2 == 1).then(|| (((n / 2) % self.threads) as usize, n / 2 / self.threads))
    }
}

/// What thread `t` knows: its own keys exactly, and lower bounds for hot
/// counts (`None`: not seen yet; hot keys are never removed).
struct Known<const N: usize> {
    own: BTreeMap<u32, W<N>>,
    hot: Vec<Option<u64>>,
    added: Vec<u64>,
}

fn get_one<K: Key, const N: usize, S: InlineKeyStrategy<K>, A: Allocator>(map: &BBTreeMap<K, W<N>, S, A>, k: &K, how: Read) -> Option<Option<W<N>>> {
    match how {
        Read::Copied => Some(map.get_copied(k)),
        Read::Cloned => Some(map.get_cloned(k)),
        Read::Read => Some(map.read(k, |_, v| *v)),
        Read::Contains => None,
    }
}

fn get_many<K: Key, const N: usize, S: InlineKeyStrategy<K>, A: Allocator, const B: usize>(map: &BBTreeMap<K, W<N>, S, A>, keys: &[K], how: Read) -> Vec<Option<W<N>>> {
    let refs: [&K; B] = std::array::from_fn(|j| &keys[j]);
    match how {
        Read::Copied => map.get_many_copied(refs).to_vec(),
        Read::Cloned => map.get_many_cloned(refs).to_vec(),
        _ => map.read_many(refs, |_, v| *v).to_vec(),
    }
}

/// A lane's result; a refused lane's entry, as it came back.
type BatchOut<K, const N: usize> = Vec<Result<Option<W<N>>, (K, W<N>)>>;

fn batch<K: Key, const N: usize, S: InlineKeyStrategy<K>, A: Allocator, const B: usize>(
    map: &BBTreeMap<K, W<N>, S, A>,
    entries: Vec<(K, W<N>)>,
    upsert: bool,
    hot: &dyn Fn(&K) -> Option<u32>,
    ran: &Cell<usize>,
) -> BatchOut<K, N> {
    let mut it = entries.into_iter();
    let arr: [(K, W<N>); B] = std::array::from_fn(|_| it.next().unwrap());
    if upsert {
        map.update_or_insert_many(arr, |k, stored, new| {
            ran.set(ran.get() + 1);
            let old = *stored;
            *stored = match hot(k) {
                Some(n) => W::new(n, old.payload().wrapping_add(new.payload())),
                None => new,
            };
            old
        })
        .into_iter()
        .map(|r| r.map_err(|e| e.into_entry()))
        .collect()
    } else {
        map.insert_many(arr).into_iter().map(|r| r.map_err(|e| e.into_entry())).collect()
    }
}

pub fn run<K: Key, S: InlineKeyStrategy<K> + Default + Send + Sync, const N: usize, const SHRINK: bool>(case: &Case) -> Result<(), Fail>
where
    BBTreeMap<K, W<N>, S>: Sync,
{
    run_in::<K, S, Global, N, SHRINK>(case, Global, &[], &|| 0)
}

/// [`run`] on a map whose pages come from `alloc`; with `refuse` (thread
/// `t` refuses as `refuse[t % len]` says, see [`Flaky`]) a refused write is
/// checked instead of failing the case. `pages_out` is what `alloc` has
/// out, for the leak check after the drop.
fn run_in<K: Key, S: InlineKeyStrategy<K> + Send + Sync, A: Allocator + Send + Sync, const N: usize, const SHRINK: bool>(
    case: &Case,
    alloc: A,
    refuse: &[Refuse],
    pages_out: &dyn Fn() -> isize,
) -> Result<(), Fail>
where
    BBTreeMap<K, W<N>, S, A>: Sync,
{
    let (own, hots, spacing, preload, plans) = case;
    let shape = Shape { threads: plans.len().max(1) as u32, own: (*own).max(1) as u32, hots: (*hots).max(1) as u32, spacing: *spacing as u32 };
    let sh = &shape;
    let lenient = !refuse.is_empty();
    #[allow(unused_mut)]
    let mut map = BBTreeMap::<K, W<N>, S, A>::new_in(alloc);
    let key = |n: u32| K::of(n, SHRINK);
    // Preload: a share of every thread's own keys, spread evenly.
    let mut initial: Vec<BTreeMap<u32, W<N>>> = vec![BTreeMap::new(); plans.len()];
    for i in 0..shape.own {
        for (t, init) in initial.iter_mut().enumerate() {
            let n = shape.own_number(t, i);
            if (i * 7 + t as u32 * 13) % 100 < *preload as u32 && !(SHRINK && K::outside(n)) && shape.hot_index(n).is_none() {
                let v = W::new(n, PRELOAD | i as u64);
                map.insert(key(n), v).map_err(|_| fail("synclin__refused__preload", format!("preload {n} refused")))?;
                init.insert(i, v);
            }
        }
    }
    let start = Barrier::new(plans.len());
    let results: Vec<Result<Known<N>, Fail>> = std::thread::scope(|scope| {
        let handles: Vec<_> = plans
            .iter()
            .zip(initial)
            .enumerate()
            .map(|(t, (plan, init))| {
                let (map, start) = (&map, &start);
                spawn_in(scope, move || -> Result<Known<N>, Fail> {
                    let mut kn = Known::<N> { own: init, hot: vec![None; sh.hots as usize], added: vec![0; sh.hots as usize] };
                    let own_i = |i: u16| i as u32 % sh.own;
                    let hot_n = |h: u8| sh.hot_number(h as u32);
                    let refusing = refuse.get(t % refuse.len().max(1)).copied();
                    let _reset = Flaky::arm(refusing.map_or(0, |r| r.seed(t)));
                    start.wait();
                    for (step, op) in plan.iter().enumerate() {
                        let ctx = |sig: &str| {
                            let sig = sig.to_string();
                            move |why: String| fail(&sig, format!("thread {t} op {step} {op:?}: {why}"))
                        };
                        if let Some(r) = refusing {
                            Flaky::set_rate(r.rate_at(step));
                        }
                        // A refused write (only with `refuse`): its entry comes
                        // back as given, its closure didn't run, and its key
                        // holds what it held.
                        macro_rules! refused {
                            ($area:literal, $sig:literal, $e:expr, $n:expr, $v:expr, $ran:expr) => {{
                                let ((rk, rv), n, v, ran): ((K, W<N>), u32, W<N>, bool) = ($e, $n, $v, $ran);
                                if !lenient {
                                    return Err(ctx($sig)("refused".into()));
                                }
                                REFUSED_OPS[0].fetch_add(1, Ordering::Relaxed);
                                ensure(rk == key(n) && rv == v, concat!("synclin__refused_entry__", $area), || {
                                    format!("thread {t} op {step}: refused entry came back as ({rk:?}, {rv:?}), given ({:?}, {v:?})", key(n))
                                })?;
                                ensure(!ran, concat!("synclin__closure_ran__", $area), || format!("thread {t} op {step}: the closure ran, yet the write of key number {n} was refused"))?;
                            }};
                        }
                        // A thread's own key, as its model has it.
                        macro_rules! expect_own {
                            ($sig:literal, $i:expr, $got:expr) => {{
                                let (i, got): (u32, Option<W<N>>) = ($i, $got);
                                let want = kn.own.get(&i).copied();
                                if got != want {
                                    return Err(ctx($sig)(format!("own key number {} (index {i}): got {got:?}, its only writer left {want:?}", sh.own_number(t, i))));
                                }
                            }};
                        }
                        // A hot count read now: whole, and no lower than
                        // this thread saw or made it before.
                        macro_rules! saw_hot {
                            ($kind:literal, $h:expr, $got:expr) => {{
                                let (h, got): (u32, Option<W<N>>) = ($h, $got);
                                let n = sh.hot_number(h);
                                match got {
                                    Some(v) => {
                                        v.check(n).map_err(ctx(concat!("synclin__torn__", $kind)))?;
                                        if let Some(prev) = kn.hot[h as usize]
                                            && v.payload() < prev
                                        {
                                            return Err(ctx(concat!("synclin__stale__", $kind))(format!("hot key number {n}: count {} after this thread saw or made {prev}", v.payload())));
                                        }
                                        kn.hot[h as usize] = Some(v.payload());
                                    }
                                    None => {
                                        if let Some(prev) = kn.hot[h as usize] {
                                            return Err(ctx(concat!("synclin__missing__", $kind))(format!("hot key number {n} (never removed, count {prev} seen) not found")));
                                        }
                                    }
                                }
                            }};
                        }
                        let any_check = |n: u32, v: &W<N>, sig: &str| -> Result<(), Fail> {
                            v.check(n).map_err(ctx(sig))?;
                            if let Some((owner, _)) = sh.owner(n) {
                                let p = v.payload();
                                let ok = p >> 40 == 0xFF || p >> 40 == owner as u64 + 1;
                                ensure(ok, sig, || format!("thread {t} op {step}: key number {n} owned by thread {owner} holds a write of thread {}", (p >> 40) as i64 - 1))?;
                            }
                            Ok(())
                        };
                        match op {
                            Op::Insert(i) => {
                                let i = own_i(*i);
                                let n = sh.own_number(t, i);
                                let v = W::new(n, own_payload(t, step));
                                match map.insert(key(n), v) {
                                    Ok(old) => {
                                        expect_own!("synclin__mismatch__insert", i, old);
                                        kn.own.insert(i, v);
                                    }
                                    Err(e) => {
                                        let existed = kn.own.contains_key(&i);
                                        if existed {
                                            REFUSED_OPS[2].fetch_add(1, Ordering::Relaxed);
                                        }
                                        refused!("insert", "synclin__refused__insert", e.into_entry(), n, v, false);
                                        expect_own!("synclin__refused_changed__insert", i, map.get_copied(&key(n)));
                                    }
                                }
                            }
                            Op::Remove(i) => {
                                let i = own_i(*i);
                                let got = map.remove(&key(sh.own_number(t, i)));
                                expect_own!("synclin__mismatch__remove", i, got);
                                kn.own.remove(&i);
                            }
                            Op::RemoveRun(a, len) => {
                                for j in 0..*len as u32 {
                                    let i = (own_i(*a) + j) % sh.own;
                                    let n = sh.own_number(t, i);
                                    let got = map.remove_entry(&key(n));
                                    if let Some((k, _)) = &got {
                                        ensure(*k == key(n), "synclin__mismatch__remove_entry", || format!("thread {t} op {step}: removed key {k:?} for {:?}", key(n)))?;
                                    }
                                    expect_own!("synclin__mismatch__remove", i, got.map(|(_, v)| v));
                                    kn.own.remove(&i);
                                }
                            }
                            Op::RemoveIf(i, parity) => {
                                let i = own_i(*i);
                                let n = sh.own_number(t, i);
                                let got = map.remove_if(&key(n), |_, v| (v.payload() & 1 == 1) == *parity).map(|(_, v)| v);
                                let want = kn.own.get(&i).copied().filter(|v| (v.payload() & 1 == 1) == *parity);
                                ensure(got == want, "synclin__mismatch__remove_if", || {
                                    format!("thread {t} op {step}: own key number {n}: removed {got:?}, its only writer left {:?} (condition: odd payload == {parity})", kn.own.get(&i))
                                })?;
                                if got.is_some() {
                                    kn.own.remove(&i);
                                }
                            }
                            Op::Update(i) => {
                                let i = own_i(*i);
                                let n = sh.own_number(t, i);
                                let v = W::new(n, own_payload(t, step));
                                let got = map.update(&key(n), |_, s| std::mem::replace(s, v));
                                expect_own!("synclin__mismatch__update", i, got);
                                if got.is_some() {
                                    kn.own.insert(i, v);
                                }
                            }
                            Op::GetOrInsert(i) => {
                                let i = own_i(*i);
                                let n = sh.own_number(t, i);
                                let v = W::new(n, own_payload(t, step));
                                let ran = Cell::new(false);
                                let got = match map.get_or_insert(key(n), v, |_, s| {
                                    ran.set(true);
                                    *s
                                }) {
                                    Ok(got) => got,
                                    Err(e) => {
                                        refused!("get_or_insert", "synclin__refused__get_or_insert", e.into_entry(), n, v, ran.get());
                                        expect_own!("synclin__refused_changed__get_or_insert", i, map.get_copied(&key(n)));
                                        continue;
                                    }
                                };
                                let want = kn.own.get(&i).copied().unwrap_or(v);
                                ensure(got == want, "synclin__mismatch__get_or_insert", || format!("thread {t} op {step}: own key number {n}: saw {got:?}, want {want:?}"))?;
                                kn.own.insert(i, want);
                            }
                            Op::Upsert(i) => {
                                let i = own_i(*i);
                                let n = sh.own_number(t, i);
                                let v = W::new(n, own_payload(t, step));
                                let ran = Cell::new(false);
                                let got = match map.update_or_insert(key(n), v, |_, s, new| {
                                    ran.set(true);
                                    std::mem::replace(s, new)
                                }) {
                                    Ok(got) => got,
                                    Err(e) => {
                                        refused!("upsert", "synclin__refused__upsert", e.into_entry(), n, v, ran.get());
                                        expect_own!("synclin__refused_changed__upsert", i, map.get_copied(&key(n)));
                                        continue;
                                    }
                                };
                                expect_own!("synclin__mismatch__upsert", i, got);
                                kn.own.insert(i, v);
                            }
                            Op::Batch(entries, upsert) => {
                                let b = match entries.len() {
                                    0 => continue,
                                    1..4 => 1,
                                    4..16 => 4,
                                    16..70 => 16,
                                    _ => 70,
                                };
                                let mut ks = Vec::new();
                                let mut picks = Vec::new();
                                for (p, d) in &entries[..b] {
                                    let p = match *p {
                                        Pick::Any(n) => Pick::Own(n as u16),
                                        p => p,
                                    };
                                    let (n, w) = match p {
                                        Pick::Hot(h) => (hot_n(h), W::new(hot_n(h), *d as u64)),
                                                                                Pick::Own(i) => (sh.own_number(t, own_i(i)), W::new(sh.own_number(t, own_i(i)), own_payload(t, step))),
                                        Pick::Any(_) => unreachable!(),
                                    };
                                    ks.push((key(n), w));
                                    picks.push((p, *d));
                                }
                                // insert_many replaces hot counts: only with upsert do hot keys add.
                                let upsert = *upsert || picks.iter().any(|(p, _)| matches!(p, Pick::Hot(_)));
                                let hot = |k: &K| {
                                    let n = k.number(SHRINK);
                                    sh.hot_index(n).map(|_| n)
                                };
                                let given: Vec<(u32, W<N>)> = ks.iter().map(|(k, w)| (k.number(SHRINK), *w)).collect();
                                let ran = Cell::new(0);
                                let out = match b {
                                    1 => batch::<K, N, S, A, 1>(map, ks, upsert, &hot, &ran),
                                    4 => batch::<K, N, S, A, 4>(map, ks, upsert, &hot, &ran),
                                    16 => batch::<K, N, S, A, 16>(map, ks, upsert, &hot, &ran),
                                    _ => batch::<K, N, S, A, 70>(map, ks, upsert, &hot, &ran),
                                };
                                // Every closure run is a lane that found its key.
                                let found = out.iter().filter(|r| matches!(r, Ok(Some(_)))).count();
                                if upsert && out.iter().any(Result::is_err) {
                                    ensure(ran.get() == found, "synclin__closure_ran__batch", || {
                                        format!("thread {t} op {step}: the closure ran {} times, for {found} lanes that found their key and {} refused", ran.get(), out.iter().filter(|r| r.is_err()).count())
                                    })?;
                                }
                                let mut refused_own = Vec::new();
                                for (((p, d), r), (n, w)) in picks.into_iter().zip(out).zip(given) {
                                    let r = match r {
                                        Ok(r) => r,
                                        Err(e) => {
                                            REFUSED_OPS[1].fetch_add(1, Ordering::Relaxed);
                                            refused!("batch", "synclin__refused__batch", e, n, w, false);
                                            if let Pick::Own(i) = p {
                                                refused_own.push(own_i(i));
                                            }
                                            continue;
                                        }
                                    };
                                    match p {
                                        Pick::Own(i) => {
                                            let i = own_i(i);
                                            expect_own!("synclin__mismatch__batch", i, r);
                                            kn.own.insert(i, W::new(sh.own_number(t, i), own_payload(t, step)));
                                        }
                                        Pick::Hot(h) => {
                                            let h = h as u32 % sh.hots;
                                            saw_hot!("batch", h, r);
                                            kn.hot[h as usize] = Some(r.map_or(0, |v| v.payload()) + d as u64);
                                            kn.added[h as usize] += d as u64;
                                        }
                                        Pick::Any(_) => {}
                                    }
                                }
                                for i in refused_own {
                                    expect_own!("synclin__refused_changed__batch", i, map.get_copied(&key(sh.own_number(t, i))));
                                }
                            }
                            Op::Get(i, how) => {
                                let i = own_i(*i);
                                let n = sh.own_number(t, i);
                                match get_one(map, &key(n), *how) {
                                    Some(got) => expect_own!("synclin__mismatch__get", i, got),
                                    None => {
                                        let got = map.contains_key(&key(n));
                                        ensure(got == kn.own.contains_key(&i), "synclin__mismatch__contains_key", || {
                                            format!("thread {t} op {step}: own key number {n}: contains_key {got}, its only writer left {:?}", kn.own.get(&i))
                                        })?;
                                    }
                                }
                            }
                            Op::GetMany(picks, how) => {
                                let b = match picks.len() {
                                    0 => continue,
                                    1..4 => 1,
                                    4..16 => 4,
                                    16..70 => 16,
                                    _ => 70,
                                };
                                let picks = &picks[..b];
                                let nums: Vec<u32> = picks
                                    .iter()
                                    .map(|p| match *p {
                                        Pick::Own(i) => sh.own_number(t, own_i(i)),
                                        Pick::Hot(h) => hot_n(h),
                                        Pick::Any(n) => n,
                                    })
                                    .collect();
                                let keys: Vec<K> = nums.iter().map(|&n| key(n)).collect();
                                let got = match b {
                                    1 => get_many::<K, N, S, A, 1>(map, &keys, *how),
                                    4 => get_many::<K, N, S, A, 4>(map, &keys, *how),
                                    16 => get_many::<K, N, S, A, 16>(map, &keys, *how),
                                    _ => get_many::<K, N, S, A, 70>(map, &keys, *how),
                                };
                                // Lanes may read a repeated hot key in any order: each
                                // read is held to what the thread knew before the
                                // batch, and the highest count is kept after it.
                                let before = kn.hot.clone();
                                let mut after = kn.hot.clone();
                                for ((p, n), v) in picks.iter().zip(&nums).zip(got) {
                                    match *p {
                                        Pick::Own(i) => expect_own!("synclin__mismatch__get_many", own_i(i), v),
                                        Pick::Hot(h) => {
                                            let h = h as u32 % sh.hots;
                                            kn.hot[h as usize] = before[h as usize];
                                            saw_hot!("get_many", h, v);
                                            after[h as usize] = after[h as usize].max(kn.hot[h as usize]);
                                        }
                                        Pick::Any(_) => {
                                            if let Some(v) = v {
                                                any_check(*n, &v, "synclin__torn__get_many")?;
                                            }
                                        }
                                    }
                                }
                                kn.hot = after;
                            }
                            Op::Add(h, d) => {
                                let h = *h as u32 % sh.hots;
                                let n = sh.hot_number(h);
                                let ran = Cell::new(false);
                                let r = match map.update_or_insert(key(n), W::new(n, *d as u64), |_, s, new| {
                                    ran.set(true);
                                    let old = *s;
                                    *s = W::new(n, old.payload() + new.payload());
                                    old
                                }) {
                                    Ok(r) => r,
                                    Err(e) => {
                                        refused!("add", "synclin__refused__add", e.into_entry(), n, W::new(n, *d as u64), ran.get());
                                        continue;
                                    }
                                };
                                saw_hot!("add", h, r);
                                kn.hot[h as usize] = Some(r.map_or(0, |v| v.payload()) + *d as u64);
                                kn.added[h as usize] += *d as u64;
                            }
                            Op::AddByUpdate(h, d) => {
                                let h = *h as u32 % sh.hots;
                                let n = sh.hot_number(h);
                                let ran = Cell::new(false);
                                let seen = match map.get_or_insert(key(n), W::new(n, 0), |_, s| {
                                    ran.set(true);
                                    *s
                                }) {
                                    Ok(seen) => seen,
                                    Err(e) => {
                                        refused!("get_or_insert", "synclin__refused__get_or_insert", e.into_entry(), n, W::new(n, 0), ran.get());
                                        continue;
                                    }
                                };
                                saw_hot!("get_or_insert", h, Some(seen));
                                let r = map.update(&key(n), |_, s| {
                                    let old = *s;
                                    *s = W::new(n, old.payload() + *d as u64);
                                    old
                                });
                                ensure(r.is_some(), "synclin__missing__update", || format!("thread {t} op {step}: hot key number {n} (never removed) not found by update"))?;
                                saw_hot!("update", h, r);
                                kn.hot[h as usize] = Some(r.map_or(0, |v| v.payload()) + *d as u64);
                                kn.added[h as usize] += *d as u64;
                            }
                            Op::Peek(h, how) => {
                                let h = *h as u32 % sh.hots;
                                let n = sh.hot_number(h);
                                match get_one(map, &key(n), *how) {
                                    Some(got) => saw_hot!("get", h, got),
                                    None => {
                                        let got = map.contains_key(&key(n));
                                        ensure(got || kn.hot[h as usize].is_none(), "synclin__missing__contains_key", || format!("thread {t} op {step}: hot key number {n} (never removed) not contained"))?;
                                    }
                                }
                            }
                            Op::Scan(a, b, rev) => {
                                let (lo, hi) = (*a, *b);
                                let mut prev: Option<K> = None;
                                let mut bad: Option<Fail> = None;
                                let mut seen: Vec<(u32, W<N>)> = Vec::new();
                                let (klo, khi) = (key(lo), key(hi));
                                let visit = |k: &K, v: &W<N>| {
                                    let ordered = prev.as_ref().is_none_or(|p| if *rev { k < p } else { k > p });
                                    if !ordered || *k < klo || *k > khi {
                                        bad = Some(ctx("synclin__order__scan")(format!("key {k:?} after {prev:?}, range {klo:?}..={khi:?}")));
                                        return ControlFlow::Break(());
                                    }
                                    let n = k.number(SHRINK);
                                    if let Err(e) = any_check(n, v, "synclin__torn__scan") {
                                        bad = Some(e);
                                        return ControlFlow::Break(());
                                    }
                                    prev = Some(k.clone());
                                    seen.push((n, *v));
                                    ControlFlow::Continue(())
                                };
                                let range = (Bound::Included(klo.clone()), Bound::Included(khi.clone()));
                                if *rev { map.scan_rev(range, visit) } else { map.scan(range, visit) };
                                if let Some(f) = bad {
                                    return Err(f);
                                }
                                walked(sh, t, &mut kn, lo, hi, &seen, "scan").map_err(|(sig, why)| ctx(sig)(why))?;
                            }
                            Op::Range(a, b, pattern) => {
                                let (lo, hi) = (*a, *b);
                                let mut it = map.range_cloned(key(lo)..=key(hi));
                                let (mut front, mut back): (Vec<(K, W<N>)>, Vec<(K, W<N>)>) = (Vec::new(), Vec::new());
                                let mut pat = pattern.iter().copied().chain(std::iter::repeat(false));
                                loop {
                                    let from_back = pat.next().unwrap_or(false);
                                    let next = if from_back { it.next_back() } else { it.next() };
                                    match next {
                                        Some(e) if from_back => back.push(e),
                                        Some(e) => front.push(e),
                                        None => break,
                                    }
                                }
                                // Not fused: it looks again past the last key it handed
                                // out, so an entry inserted since may still come.
                                drop(it);
                                back.reverse();
                                front.extend(back);
                                let mut seen = Vec::new();
                                for (j, (k, v)) in front.iter().enumerate() {
                                    if j > 0 && *k <= front[j - 1].0 || *k < key(lo) || *k > key(hi) {
                                        return Err(ctx("synclin__order__range")(format!("range_cloned handed {k:?} after {:?} (front and back joined)", j.checked_sub(1).map(|p| &front[p].0))));
                                    }
                                    let n = k.number(SHRINK);
                                    any_check(n, v, "synclin__torn__range")?;
                                    seen.push((n, *v));
                                }
                                walked(sh, t, &mut kn, lo, hi, &seen, "range").map_err(|(sig, why)| ctx(sig)(why))?;
                            }
                            Op::IsEmpty => {
                                let known = !kn.own.is_empty() || kn.hot.iter().any(Option::is_some);
                                let got = map.is_empty();
                                ensure(!(known && got), "synclin__mismatch__is_empty", || format!("thread {t} op {step}: is_empty while this thread's {} own keys are in", kn.own.len()))?;
                            }
                        }
                    }
                    Ok(kn)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| {
                h.join().unwrap_or_else(|p| {
                    let msg = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()));
                    Err(fail("synclin__panic__thread", format!("thread panicked: {msg:?}")))
                })
            })
            .collect()
    });
    let mut known = Vec::new();
    for r in results {
        known.push(r?);
    }
    // After the join: every thread's own keys as it left them, each hot
    // key holding every thread's additions, nothing else.
    let mut want: BTreeMap<u32, W<N>> = BTreeMap::new();
    for (t, kn) in known.iter().enumerate() {
        for (&i, &v) in &kn.own {
            want.insert(shape.own_number(t, i), v);
        }
    }
    for h in 0..shape.hots {
        let total: u64 = known.iter().map(|kn| kn.added[h as usize]).sum();
        let touched = known.iter().any(|kn| kn.hot[h as usize].is_some());
        if touched {
            want.insert(shape.hot_number(h), W::new(shape.hot_number(h), total));
        }
    }
    let mut got: BTreeMap<u32, W<N>> = BTreeMap::new();
    map.scan::<K, ()>(.., |k, v| {
        got.insert(k.number(SHRINK), *v);
        ControlFlow::Continue(())
    });
    for h in 0..shape.hots {
        let n = shape.hot_number(h);
        if let (Some(w), g) = (want.get(&n), got.get(&n)) {
            ensure(g == Some(w), "synclin__lost__add", || format!("hot key number {n} after the join: {g:?}, the threads added {}", w.payload()))?;
        }
    }
    if got != want {
        let diff = want.iter().find(|(n, v)| got.get(n) != Some(v)).map(|(n, v)| format!("key number {n}: want {v:?}, got {:?}", got.get(n)));
        let extra = got.iter().find(|(n, _)| !want.contains_key(n)).map(|(n, v)| format!("extra key number {n}: {v:?}"));
        return Err(fail("synclin__final__contents", format!("after the join ({} entries, want {}): {diff:?} {extra:?}", got.len(), want.len())));
    }
    #[cfg(fuzzing)]
    {
        let count = got.len();
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| map.check_invariants(count)));
        if let Err(p) = r {
            let msg = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()));
            return Err(fail("synclin__invariant__check", format!("after the join ({count} entries): {msg:?}")));
        }
    }
    drop(map);
    let out = pages_out();
    ensure(out == 0, "synclin__leak__pages", || format!("{out} pages out after the map was dropped"))?;
    Ok(())
}

/// A walk over key numbers `lo..=hi` (ascending in `seen`): this thread's
/// own keys exactly as its model has them (each present, or absent, for the
/// whole walk), every hot key it has seen, at no lower a count.
fn walked<const N: usize>(sh: &Shape, t: usize, kn: &mut Known<N>, lo: u32, hi: u32, seen: &[(u32, W<N>)], what: &str) -> Result<(), (&'static str, String)> {
    let sig = |kind: &str| -> &'static str {
        match (kind, what) {
            ("mismatch", "scan") => "synclin__mismatch__scan",
            ("mismatch", _) => "synclin__mismatch__range",
            ("missing", "scan") => "synclin__missing__scan",
            ("missing", _) => "synclin__missing__range",
            ("stale", "scan") => "synclin__stale__scan",
            _ => "synclin__stale__range",
        }
    };
    let mine: BTreeMap<u32, W<N>> = seen.iter().filter(|(n, _)| sh.owner(*n).is_some_and(|(o, _)| o == t)).copied().collect();
    let want: BTreeMap<u32, W<N>> = kn.own.iter().map(|(&i, &v)| (sh.own_number(t, i), v)).filter(|(n, _)| (lo..=hi).contains(n)).collect();
    if mine != want {
        let missing = want.iter().find(|(n, v)| mine.get(n) != Some(v)).map(|(n, v)| format!("own key number {n} {v:?} handed out as {:?}", mine.get(n)));
        let extra = mine.iter().find(|(n, _)| !want.contains_key(n)).map(|(n, v)| format!("own key number {n} (removed by this thread) handed out: {v:?}"));
        return Err((sig("mismatch"), format!("{what} over {lo}..={hi}: {} own entries, want {}; {missing:?} {extra:?}", mine.len(), want.len())));
    }
    for h in 0..sh.hots {
        let n = sh.hot_number(h);
        if !(lo..=hi).contains(&n) {
            continue;
        }
        let got = seen.iter().find(|(m, _)| *m == n);
        match (got, kn.hot[h as usize]) {
            (None, Some(prev)) => return Err((sig("missing"), format!("{what} over {lo}..={hi}: hot key number {n} (never removed, count {prev} seen) not handed out"))),
            (Some((_, v)), Some(prev)) if v.payload() < prev => {
                return Err((sig("stale"), format!("{what} over {lo}..={hi}: hot key number {n}: count {} after this thread saw or made {prev}", v.payload())));
            }
            (Some((_, v)), _) => kn.hot[h as usize] = Some(v.payload()),
            (None, None) => {}
        }
    }
    Ok(())
}

// ---------- synclinr: the same threads on an allocator that refuses pages ----------

/// Refused writes of the case running, refused batch lanes, refused
/// inserts of a key the map held (labels, counted on the threads).
static REFUSED_OPS: [std::sync::atomic::AtomicUsize; 3] = [const { std::sync::atomic::AtomicUsize::new(0) }; 3];

thread_local! {
    /// The calling thread's refusals: out of 1024 allocations, and the
    /// state of its xorshift.
    static REFUSING: Cell<(u32, u64)> = const { Cell::new((0, 0)) };
}

/// The pages of one map, from the global allocator, refused to each thread
/// at its own rate ([`Flaky::set_rate`]): while one thread's splits fail,
/// other threads' go through, and the free pages removals leave go to
/// whichever thread asks first (`tree/latched/pages.rs`). Freed pages are
/// scribbled over.
#[derive(Clone, Default)]
pub struct Flaky(Arc<AtomicIsize>);

impl Flaky {
    pub(crate) fn out(&self) -> isize {
        self.0.load(Ordering::SeqCst)
    }
    /// Seeds this thread's refusals; refuses nothing until `set_rate`,
    /// and again after the guard drops.
    pub(crate) fn arm(seed: u64) -> impl Drop {
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                REFUSING.with(|c| c.set((0, 0)));
            }
        }
        REFUSING.with(|c| c.set((0, seed | 1)));
        Reset
    }
    pub(crate) fn set_rate(rate: u32) {
        REFUSING.with(|c| c.set((rate, c.get().1)));
    }
    fn refuse() -> bool {
        REFUSING.with(|c| {
            let (rate, mut x) = c.get();
            if rate == 0 {
                return false;
            }
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            c.set((rate, x));
            (x >> 20) % 1024 < rate as u64
        })
    }
}

unsafe impl Allocator for Flaky {
    fn allocate_zeroed(&self, layout: Layout) -> Result<NonNull<u8>, AllocError> {
        if Self::refuse() {
            return Err(AllocError);
        }
        self.0.fetch_add(1, Ordering::SeqCst);
        Global.allocate_zeroed(layout)
    }
    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        self.0.fetch_sub(1, Ordering::SeqCst);
        unsafe {
            ptr.write_bytes(0xA5, layout.size());
            Global.deallocate(ptr, layout);
        }
    }
}

/// How a thread refuses: `rate` of 1024 allocations, during the first `on`
/// of every `on + off` ops (always, when `off` is 0).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Refuse {
    rate: u16,
    on: u8,
    off: u8,
    seed: u16,
}

impl Refuse {
    pub(crate) const NONE: Refuse = Refuse { rate: 0, on: 1, off: 0, seed: 0 };
    pub(crate) fn seed(&self, t: usize) -> u64 {
        t as u64 * 0x9E37_79B9 + self.seed as u64
    }
    pub(crate) fn rate_at(&self, step: usize) -> u32 {
        let period = self.on as usize + self.off as usize;
        if self.off == 0 || step % period < self.on as usize { self.rate.min(1024) as u32 } else { 0 }
    }
}

/// A synclin case, and how each thread refuses (thread `t` by entry `t % len`).
pub type RCase = (Case, Vec<Refuse>);

pub fn rcase() -> BoxGen<RCase> {
    (case(), refuses()).boxed()
}

/// How the threads of a case refuse: 1 to 6 [`Refuse`]s.
pub fn refuses() -> BoxGen<Vec<Refuse>> {
    let rate = gens::frequency(vec![
        (1, gens::constant(0u16).boxed()),
        (2, gens::int(1u16..=16).boxed()),
        (2, gens::int(16u16..=200).boxed()),
        (1, gens::int(200u16..=1023).boxed()),
        (1, gens::constant(1024u16).boxed()),
    ])
    .boxed();
    let window = gens::frequency(vec![(2, gens::constant((1u8, 0u8)).boxed()), (1, (gens::int(1u8..=40), gens::int(1u8..=200)).boxed())]).boxed();
    let refuse = (rate, window, gens::int(0u16..=u16::MAX)).map(|(rate, (on, off), seed)| Refuse { rate, on, off, seed }).boxed();
    gens::vec(refuse).len(1..=6).boxed()
}

fn retag(mut f: Fail) -> Fail {
    if let Some(rest) = f.sig.strip_prefix("synclin__") {
        f.sig = format!("synclinr__{rest}");
    }
    f
}

/// synclin on a [`Flaky`] allocator: refused writes hand their entry back,
/// run no closure and leave the key as it was (`InsertError`: "the map is
/// unchanged, and the entry comes back"; `update_or_insert`: "`f` doesn't
/// run then"); everything else as in synclin, and every page back after the
/// drop. Signatures `synclinr__<kind>__<area>`.
pub fn run_refuse<K: Key, S: InlineKeyStrategy<K> + Send + Sync, const N: usize, const SHRINK: bool>(case: &RCase) -> Result<(), Fail>
where
    BBTreeMap<K, W<N>, S, Flaky>: Sync,
{
    let (case, refuse) = case;
    let alloc = Flaky::default();
    let pages = alloc.clone();
    let refuse = if refuse.is_empty() { vec![Refuse::NONE] } else { refuse.clone() };
    for c in &REFUSED_OPS {
        c.store(0, Ordering::Relaxed);
    }
    run_in::<K, S, Flaky, N, SHRINK>(case, alloc, &refuse, &|| pages.out()).map_err(retag)?;
    let n = REFUSED_OPS.each_ref().map(|c| c.load(Ordering::Relaxed));
    classify(n[0] > 0, "refused");
    classify(n[0] >= 100, "refused_100");
    classify(n[1] > 0, "refused_in_batch");
    classify(n[2] > 0, "refused_existing");
    Ok(())
}
