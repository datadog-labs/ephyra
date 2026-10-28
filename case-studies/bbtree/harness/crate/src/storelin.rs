// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! `bbtree-store` shared by 2 to 6 threads, checked while they run. Each
//! thread owns its own keys, interleaved with every other thread's (key
//! number `i * slots + t`), so their leaves are shared: a leaf's records lie
//! in heap chains of leaves other threads hold (`leaf_heap.rs`: "A record of
//! another leaf may lie in a leaf's chain, once entries move between
//! leaves; a thread holding that other leaf kills it"), compactions and
//! orphans happen under all of them, yet no other thread changes a thread's
//! keys: every answer about them has one right value, from its own model,
//! at every step (insert's "whether it replaced a value", remove's "whether
//! the store held it", get, get_into "as `Read::read_to_end` does", with
//! `buf` unchanged on `None`, contains_key, scan / scan_rev / range from
//! both ends: "an entry present for the whole scan is visited exactly once").
//! Pages are refused to one thread at a time, inside one of its inserts
//! (a thread-local budget): "The store holds what it held then", so its key
//! keeps its old value. Shared hot keys (never removed, preloaded) take
//! self-describing values from every thread and must stay whole and present.
//! After the join, the whole store is the union of the models, and after
//! the drop no page is out. Nondeterministic: repro tries several times.

use std::alloc::Layout;
use std::cell::Cell;
use std::collections::BTreeMap;
use std::ops::{Bound, ControlFlow};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};

use bbtree_store::bbtree::{AllocError, Allocator, Global, NodeConfig};
use bbtree_store::{BBTreeStore, InsertError};
use ephyra::{BoxGen, GenExt, classify, gens};
use serde::{Deserialize, Serialize};

use crate::common::{Fail, ensure, fail, spawn_in};

thread_local! {
    /// Pages this thread may still take (`usize::MAX`: any number).
    static ALLOW: Cell<usize> = const { Cell::new(usize::MAX) };
    /// Whether this thread was refused a page since the flag was cleared.
    static REFUSED: Cell<bool> = const { Cell::new(false) };
}

/// Counts pages out, refuses pages to a thread whose budget ran out, and
/// scribbles over pages given back.
#[derive(Clone, Default)]
pub struct Refusing(Arc<AtomicUsize>);

unsafe impl Allocator for Refusing {
    fn allocate_zeroed(&self, layout: Layout) -> Result<NonNull<u8>, AllocError> {
        let allow = ALLOW.get();
        if allow == 0 {
            REFUSED.set(true);
            return Err(AllocError);
        }
        if allow != usize::MAX {
            ALLOW.set(allow - 1);
        }
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

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Op {
    /// Own key index (taken modulo the own count), value length.
    Insert(u16, u32),
    /// The same, with `allow` pages for the thread before it is refused.
    Refused(u16, u32, u8),
    Remove(u16),
    Get(u16),
    /// Into a buffer holding `prefill` bytes already.
    GetInto(u16, u8),
    Contains(u16),
    /// Own keys from the index on, inserted / removed in a row: whole
    /// leaves fill and empty, so they split and merge under the others.
    InsertRun(u16, u8, u32),
    RemoveRun(u16, u8),
    /// Global key numbers (modulo the span); `stub` bounds are the key
    /// number alone, without the key's fill; `excl` bit 0 excludes the
    /// start, bit 1 the end; `limit` > 0 breaks after that many entries.
    Scan { from: u16, to: u16, rev: bool, limit: u16, stub: bool, excl: u8 },
    Range { from: u16, to: u16, back_every: u8, stub: bool, excl: u8 },
    Hot(u8, u32),
    HotGet(u8),
}

fn op() -> BoxGen<Op> {
    let i = gens::int(0u16..=u16::MAX);
    let g = gens::int(0u16..=u16::MAX);
    let len = gens::frequency(vec![
        (5, gens::int(0u32..=40).boxed()),
        (3, gens::int(0u32..=600).boxed()),
        (2, gens::int(200u32..=1200).boxed()),
        (1, gens::int(1200u32..=20_000).boxed()),
    ]);
    gens::frequency(vec![
        (8, (i.clone(), len.clone()).map(|(i, l)| Op::Insert(i, l)).boxed()),
        (2, (i.clone(), len.clone(), gens::int(0u8..=3)).map(|(i, l, a)| Op::Refused(i, l, a)).boxed()),
        (4, i.clone().map(Op::Remove).boxed()),
        (3, i.clone().map(Op::Get).boxed()),
        (1, (i.clone(), gens::int(0u8..=20)).map(|(i, p)| Op::GetInto(i, p)).boxed()),
        (1, i.clone().map(Op::Contains).boxed()),
        (1, (i.clone(), gens::int(1u8..=80), len.clone()).map(|(i, n, l)| Op::InsertRun(i, n, l)).boxed()),
        (1, (i.clone(), gens::int(1u8..=120)).map(|(i, n)| Op::RemoveRun(i, n)).boxed()),
        (
            1,
            (g.clone(), g.clone(), gens::bool(), gens::frequency(vec![(3, gens::constant(0u16).boxed()), (1, gens::int(1u16..=20).boxed())]), gens::bool(), gens::int(0u8..=3))
                .map(|(from, to, rev, limit, stub, excl)| Op::Scan { from, to, rev, limit, stub, excl })
                .boxed(),
        ),
        (
            1,
            (g.clone(), g, gens::int(0u8..=4), gens::bool(), gens::int(0u8..=3))
                .map(|(from, to, back_every, stub, excl)| Op::Range { from, to, back_every, stub, excl })
                .boxed(),
        ),
        (1, (gens::int(0u8..=u8::MAX), len).map(|(h, l)| Op::Hot(h, l)).boxed()),
        (1, gens::int(0u8..=u8::MAX).map(Op::HotGet).boxed()),
    ])
    .boxed()
}

/// Own keys per thread, hot keys, key shape (0 short, 1 mixed with some
/// near the longest, 2 long, 3 behind a 30-byte shared prefix), preloaded
/// percent of own keys, one plan per thread.
pub type Case = (u16, u8, u8, u8, Vec<Vec<Op>>);

pub fn case() -> BoxGen<Case> {
    (
        gens::frequency(vec![(1, gens::int(1u16..=16).boxed()), (2, gens::int(16u16..=300).boxed()), (2, gens::int(300u16..=3000).boxed())]),
        gens::int(0u8..=8),
        gens::int(0u8..=3),
        gens::frequency(vec![(1, gens::constant(0u8).boxed()), (2, gens::int(0u8..=100).boxed())]),
        gens::vec(gens::vec(op()).len(0..=300)).len(2..=6),
    )
        .boxed()
}

/// How keys look in one case.
struct Shape {
    threads: usize,
    /// Threads, plus one for the hot keys when there are any.
    slots: u32,
    own: u32,
    hots: u32,
    mode: u8,
    max_key: usize,
}

impl Shape {
    /// Where the key number lies in a key.
    fn at(&self) -> usize {
        if self.mode == 3 { 31 } else { 1 }
    }
    fn stub(&self, g: u32) -> Vec<u8> {
        let mut k = vec![b'k'];
        if self.mode == 3 {
            k.extend(std::iter::repeat_n(b'P', 30));
        }
        k.extend_from_slice(&g.to_be_bytes());
        k
    }
    /// Key number `g`: the stub, then a fill whose length depends on `g`.
    fn key(&self, g: u32) -> Vec<u8> {
        let mut k = self.stub(g);
        let room = self.max_key - k.len();
        let fill = match self.mode {
            0 | 3 => (g as usize * 7) % 12,
            1 if g % 5 == 0 => room - (g as usize % 9),
            1 => (g as usize * 7) % 23,
            _ => room - (g as usize % 40).min(room),
        };
        k.extend((0..fill).map(|j| b'a' + ((g as usize + j) % 26) as u8));
        k
    }
    fn number(&self, k: &[u8]) -> Option<u32> {
        let at = self.at();
        let bytes = k.get(at..at + 4)?;
        Some(u32::from_be_bytes(bytes.try_into().ok()?))
    }
    /// The thread owning key number `g`; `threads` for a hot key.
    fn owner(&self, g: u32) -> usize {
        (g % self.slots) as usize
    }
    fn own_key(&self, t: usize, i: u32) -> u32 {
        (i % self.own) * self.slots + t as u32
    }
    fn hot_key(&self, h: u32) -> u32 {
        (h % self.hots) * self.slots + self.threads as u32
    }
    fn span(&self) -> u32 {
        self.own * self.slots
    }
}

/// The value of key number `g`, written by `version`, `len` bytes long (at
/// least its 12-byte header): key number, version, length, then a fill.
fn value(g: u32, version: u32, len: u32) -> Vec<u8> {
    let len = len.max(12) as usize;
    let mut v = Vec::with_capacity(len);
    v.extend_from_slice(&g.to_le_bytes());
    v.extend_from_slice(&version.to_le_bytes());
    v.extend_from_slice(&(len as u32).to_le_bytes());
    let seed = g.wrapping_mul(2654435761) ^ version;
    v.extend((12..len).map(|i| (seed.wrapping_add(i as u32).wrapping_mul(0x9E37) >> 7) as u8));
    v
}

/// Whether `v` is a value `value` made for key number `g`; why not, if not.
fn check_value(g: u32, v: &[u8]) -> Result<(), String> {
    if v.len() < 12 {
        return Err(format!("key number {g}: value of {} bytes, shorter than its header", v.len()));
    }
    let word = |i: usize| u32::from_le_bytes(v[i..i + 4].try_into().unwrap());
    let (n, version, len) = (word(0), word(4), word(8));
    if n != g {
        return Err(format!("key number {g} holds a value of key number {n}"));
    }
    let want = value(n, version, len);
    if len as usize != v.len() || want != v {
        let at = want.iter().zip(v).position(|(a, b)| a != b);
        return Err(format!("value of key number {g} version {version:#x}: {} bytes, header says {len}, first wrong byte at {at:?}", v.len()));
    }
    Ok(())
}

fn short(v: Option<&[u8]>) -> String {
    match v {
        None => "none".into(),
        Some(v) if v.len() >= 12 => {
            let word = |i: usize| u32::from_le_bytes(v[i..i + 4].try_into().unwrap());
            format!("{} bytes (key number {}, version {:#x})", v.len(), word(0), word(4))
        }
        Some(v) => format!("{} bytes {v:?}", v.len()),
    }
}

type Model = BTreeMap<Vec<u8>, Vec<u8>>;

/// Range bounds for global numbers `from` and `to` that never panic.
fn bounds(sh: &Shape, from: u16, to: u16, stub: bool, excl: u8) -> (Bound<Vec<u8>>, Bound<Vec<u8>>) {
    let span = sh.span().max(1);
    let (a, b) = (from as u32 % span, to as u32 % span);
    let (lo, hi) = (a.min(b), a.max(b));
    let k = |g| if stub { sh.stub(g) } else { sh.key(g) };
    let mut excl = excl;
    if lo == hi && excl == 3 {
        excl = 1;
    }
    let start = if excl & 1 != 0 { Bound::Excluded(k(lo)) } else { Bound::Included(k(lo)) };
    let end = if excl & 2 != 0 { Bound::Excluded(k(hi)) } else { Bound::Included(k(hi)) };
    (start, end)
}

fn within(k: &[u8], b: &(Bound<Vec<u8>>, Bound<Vec<u8>>)) -> bool {
    let after = match &b.0 {
        Bound::Included(s) => k >= &s[..],
        Bound::Excluded(s) => k > &s[..],
        Bound::Unbounded => true,
    };
    let before = match &b.1 {
        Bound::Included(e) => k <= &e[..],
        Bound::Excluded(e) => k < &e[..],
        Bound::Unbounded => true,
    };
    after && before
}

fn as_ref(b: &(Bound<Vec<u8>>, Bound<Vec<u8>>)) -> (Bound<&[u8]>, Bound<&[u8]>) {
    (b.0.as_ref().map(|k| &k[..]), b.1.as_ref().map(|k| &k[..]))
}

/// Checks a walk's entries (in the order handed out) against `t`'s model:
/// every entry whole and in range, `t`'s keys in the range exactly the
/// model's (up to `upto`, the last key a broken walk reached, from `rev`'s
/// side). Returns the signature kind and why, on a failure.
fn walked(sh: &Shape, t: usize, model: &Model, b: &(Bound<Vec<u8>>, Bound<Vec<u8>>), seen: &[(Vec<u8>, Vec<u8>)], upto: Option<(&[u8], bool)>) -> Result<(), (&'static str, String)> {
    let mut mine = Vec::new();
    for (k, v) in seen {
        if !within(k, b) {
            return Err(("order", format!("key {} handed out, outside the range", short(Some(k)))));
        }
        let Some(g) = sh.number(k).filter(|&g| sh.key(g) == *k) else {
            return Err(("torn", format!("handed out a key no thread writes: {k:?}")));
        };
        check_value(g, v).map_err(|e| ("torn", e))?;
        if sh.owner(g) == t {
            mine.push((k.clone(), v.clone()));
        }
    }
    mine.sort();
    let want: Vec<(Vec<u8>, Vec<u8>)> = model
        .range::<[u8], _>(as_ref(b))
        .filter(|(k, _)| match upto {
            None => true,
            Some((last, false)) => &k[..] <= last,
            Some((last, true)) => &k[..] >= last,
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    if mine != want {
        let missing: Vec<_> = want.iter().filter(|e| !mine.contains(e)).map(|(k, v)| (sh.number(k), short(Some(v)))).take(4).collect();
        let extra: Vec<_> = mine.iter().filter(|e| !want.contains(e)).map(|(k, v)| (sh.number(k), short(Some(v)))).take(4).collect();
        return Err(("mismatch", format!("own keys: {} handed out, {} in the model; missing {missing:?}, unexpected {extra:?}", mine.len(), want.len())));
    }
    Ok(())
}

struct Tally {
    refused: u64,
}

pub fn run<C: NodeConfig + Default + Send + Sync + 'static>(case: &Case) -> Result<(), Fail>
where
    BBTreeStore<Refusing, C>: Sync,
{
    let (own, hots, mode, preload, plans) = case;
    let threads = plans.len();
    let alloc = Refusing::default();
    let mut refused_total = 0;
    {
        let store = BBTreeStore::with_config_in(C::default(), alloc.clone());
        let sh = Shape {
            threads,
            slots: threads as u32 + u32::from(*hots > 0),
            own: u32::from((*own).max(1)),
            hots: u32::from(*hots),
            mode: *mode,
            max_key: store.max_key_len(),
        };
        let mut models: Vec<Model> = vec![Model::new(); threads];
        for (t, model) in models.iter_mut().enumerate() {
            for i in 0..sh.own {
                if (i * 37 + t as u32 * 11) % 100 < u32::from(*preload) {
                    let g = sh.own_key(t, i);
                    let (k, v) = (sh.key(g), value(g, 0, g * 13 % 1500));
                    store.insert(&k, &v).map_err(|e| fail("storelin__error__preload", format!("preload of key number {g}: {e}")))?;
                    model.insert(k, v);
                }
            }
        }
        for h in 0..sh.hots {
            let g = sh.hot_key(h);
            store.insert(&sh.key(g), &value(g, 0, 40)).map_err(|e| fail("storelin__error__preload", format!("preload of hot key number {g}: {e}")))?;
        }
        let start = Barrier::new(threads);
        let outcomes: Vec<Result<(Model, Tally), Fail>> = std::thread::scope(|scope| {
            let handles: Vec<_> = plans
                .iter()
                .zip(models)
                .enumerate()
                .map(|(t, (plan, mut model))| {
                    let (store, start, sh) = (&store, &start, &sh);
                    spawn_in(scope, move || -> Result<(Model, Tally), Fail> {
                        let mut tally = Tally { refused: 0 };
                        start.wait();
                        for (i, op) in plan.iter().enumerate() {
                            let ctx = |sig: &str, why: String| fail(&format!("storelin__{sig}"), format!("thread {t} of {threads}, op {i} {op:?}: {why}"));
                            let version = (t as u32) << 20 | (i as u32 & 0xF_FFFF);
                            match *op {
                                Op::Insert(n, len) => {
                                    let g = sh.own_key(t, n.into());
                                    let (k, v) = (sh.key(g), value(g, version, len));
                                    let had = model.contains_key(&k);
                                    match store.insert(&k, &v) {
                                        Ok(replaced) => ensure(replaced == had, "storelin__mismatch__insert", || format!("thread {t} op {i} {op:?}: insert of key number {g} returned replaced = {replaced}, the store held it: {had}"))?,
                                        Err(e) => return Err(ctx("error__insert", format!("key number {g}: {e}"))),
                                    }
                                    model.insert(k, v);
                                }
                                Op::Refused(n, len, allow) => {
                                    let g = sh.own_key(t, n.into());
                                    let (k, v) = (sh.key(g), value(g, version, len));
                                    let had = model.contains_key(&k);
                                    REFUSED.set(false);
                                    ALLOW.set(allow.into());
                                    let got = store.insert(&k, &v);
                                    ALLOW.set(usize::MAX);
                                    match got {
                                        Ok(replaced) => {
                                            ensure(replaced == had, "storelin__mismatch__insert", || format!("thread {t} op {i} {op:?}: insert of key number {g} returned replaced = {replaced}, the store held it: {had}"))?;
                                            model.insert(k.clone(), v);
                                        }
                                        Err(InsertError::Alloc(_)) if REFUSED.get() => tally.refused += 1,
                                        Err(e) => return Err(ctx("error__refused", format!("key number {g}: {e}, refused a page: {}", REFUSED.get()))),
                                    }
                                    let now = store.get(&k);
                                    if now.as_ref() != model.get(&k) {
                                        return Err(ctx("mismatch__refused", format!("key number {g} after the insert ({got:?}): {}, the model: {}", short(now.as_deref()), short(model.get(&k).map(|v| &v[..])))));
                                    }
                                }
                                Op::Remove(n) => {
                                    let g = sh.own_key(t, n.into());
                                    let k = sh.key(g);
                                    let had = model.remove(&k).is_some();
                                    let removed = store.remove(&k);
                                    ensure(removed == had, "storelin__mismatch__remove", || format!("thread {t} op {i} {op:?}: remove of key number {g} returned {removed}, the store held it: {had}"))?;
                                }
                                Op::Get(n) => {
                                    let g = sh.own_key(t, n.into());
                                    let k = sh.key(g);
                                    let got = store.get(&k);
                                    if got.as_ref() != model.get(&k) {
                                        let torn = got.as_deref().and_then(|v| check_value(g, v).err());
                                        return Err(ctx(if torn.is_some() { "torn__get" } else { "mismatch__get" }, format!("key number {g}: {} ({torn:?}), the model: {}", short(got.as_deref()), short(model.get(&k).map(|v| &v[..])))));
                                    }
                                }
                                Op::GetInto(n, prefill) => {
                                    let g = sh.own_key(t, n.into());
                                    let k = sh.key(g);
                                    let junk: Vec<u8> = (0..prefill).map(|b| b ^ 0x5A).collect();
                                    let mut buf = junk.clone();
                                    let got = store.get_into(&k, &mut buf);
                                    let want = model.get(&k);
                                    let mut want_buf = junk;
                                    if let Some(v) = want {
                                        want_buf.extend_from_slice(v);
                                    }
                                    if got != want.map(|v| v.len()) || buf != want_buf {
                                        return Err(ctx("mismatch__get_into", format!("key number {g}: returned {got:?}, buffer {} bytes (prefill {prefill}), the model: {}", buf.len(), short(want.map(|v| &v[..])))));
                                    }
                                }
                                Op::Contains(n) => {
                                    let g = sh.own_key(t, n.into());
                                    let k = sh.key(g);
                                    let got = store.contains_key(&k);
                                    ensure(got == model.contains_key(&k), "storelin__mismatch__contains", || format!("thread {t} op {i} {op:?}: contains_key of key number {g} = {got}, the model: {}", !got))?;
                                }
                                Op::InsertRun(n, count, len) => {
                                    for j in 0..u32::from(count).min(sh.own) {
                                        let g = sh.own_key(t, u32::from(n) + j);
                                        let (k, v) = (sh.key(g), value(g, version, len.wrapping_add(j * 97) % 2400));
                                        let had = model.contains_key(&k);
                                        match store.insert(&k, &v) {
                                            Ok(replaced) => ensure(replaced == had, "storelin__mismatch__insert", || format!("thread {t} op {i} {op:?}: insert of key number {g} returned replaced = {replaced}, the store held it: {had}"))?,
                                            Err(e) => return Err(ctx("error__insert", format!("key number {g}: {e}"))),
                                        }
                                        model.insert(k, v);
                                    }
                                }
                                Op::RemoveRun(n, count) => {
                                    for j in 0..u32::from(count).min(sh.own) {
                                        let g = sh.own_key(t, u32::from(n) + j);
                                        let k = sh.key(g);
                                        let had = model.remove(&k).is_some();
                                        let removed = store.remove(&k);
                                        ensure(removed == had, "storelin__mismatch__remove", || format!("thread {t} op {i} {op:?}: remove of key number {g} returned {removed}, the store held it: {had}"))?;
                                    }
                                }
                                Op::Scan { from, to, rev, limit, stub, excl } => {
                                    let b = bounds(sh, from, to, stub, excl);
                                    let mut seen: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
                                    let mut unordered = None;
                                    let visit = |k: &[u8], v: &[u8]| {
                                        if let Some((p, _)) = seen.last() {
                                            if if rev { k >= &p[..] } else { k <= &p[..] } {
                                                unordered = Some(format!("key {:?} after {:?}", sh.number(k), sh.number(p)));
                                                return ControlFlow::Break(());
                                            }
                                        }
                                        seen.push((k.to_vec(), v.to_vec()));
                                        if limit > 0 && seen.len() >= limit as usize { ControlFlow::Break(()) } else { ControlFlow::Continue(()) }
                                    };
                                    let broke = if rev { store.scan_rev::<[u8], ()>(as_ref(&b), visit) } else { store.scan::<[u8], ()>(as_ref(&b), visit) };
                                    if let Some(why) = unordered {
                                        return Err(ctx("order__scan", why));
                                    }
                                    let upto = broke.and(seen.last()).map(|(k, _)| (&k[..], rev));
                                    walked(sh, t, &model, &b, &seen, upto).map_err(|(kind, why)| ctx(&format!("{kind}__scan"), why))?;
                                }
                                Op::Range { from, to, back_every, stub, excl } => {
                                    let b = bounds(sh, from, to, stub, excl);
                                    let mut it = store.range::<[u8], _>(as_ref(&b));
                                    let (mut front, mut back): (Option<Vec<u8>>, Option<Vec<u8>>) = (None, None);
                                    let mut seen = Vec::new();
                                    let mut step = 0u32;
                                    loop {
                                        step += 1;
                                        let from_back = back_every != 0 && step % u32::from(back_every) == 0;
                                        let Some((k, v)) = (if from_back { it.next_back() } else { it.next() }) else { break };
                                        let after_front = front.as_ref().is_none_or(|f| &k > f);
                                        let before_back = back.as_ref().is_none_or(|bk| &k < bk);
                                        if !after_front || !before_back {
                                            return Err(ctx("order__range", format!("range handed key number {:?} ({}), front at {:?}, back at {:?}", sh.number(&k), if from_back { "next_back" } else { "next" }, front.as_deref().and_then(|f| sh.number(f)), back.as_deref().and_then(|bk| sh.number(bk)))));
                                        }
                                        if from_back { back = Some(k.clone()) } else { front = Some(k.clone()) }
                                        seen.push((k, v));
                                        if step > 200_000 {
                                            return Err(ctx("runaway__range", "range handed out more than 200000 entries".into()));
                                        }
                                    }
                                    walked(sh, t, &model, &b, &seen, None).map_err(|(kind, why)| ctx(&format!("{kind}__range"), why))?;
                                }
                                Op::Hot(h, len) => {
                                    if sh.hots == 0 {
                                        continue;
                                    }
                                    let g = sh.hot_key(h.into());
                                    match store.insert(&sh.key(g), &value(g, version, len)) {
                                        Ok(true) => {}
                                        Ok(false) => return Err(ctx("missing__hot", format!("insert of hot key number {g} (preloaded, never removed) replaced nothing"))),
                                        Err(e) => return Err(ctx("error__insert", format!("hot key number {g}: {e}"))),
                                    }
                                }
                                Op::HotGet(h) => {
                                    if sh.hots == 0 {
                                        continue;
                                    }
                                    let g = sh.hot_key(h.into());
                                    match store.get(&sh.key(g)) {
                                        Some(v) => check_value(g, &v).map_err(|e| ctx("torn__hot", e))?,
                                        None => return Err(ctx("missing__hot", format!("hot key number {g} (preloaded, never removed) not found"))),
                                    }
                                }
                            }
                        }
                        Ok((model, tally))
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| {
                    h.join().unwrap_or_else(|p| {
                        let msg = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()));
                        Err(fail("storelin__panic__thread", format!("thread panicked: {msg:?}")))
                    })
                })
                .collect()
        });
        let mut all = Model::new();
        for outcome in outcomes {
            let (model, tally) = outcome?;
            refused_total += tally.refused;
            all.extend(model);
        }
        // The whole store: every entry whole, the threads' keys as their
        // models left them, every hot key there.
        let mut got = Model::new();
        let mut hot_seen = 0;
        let mut bad = None;
        store.scan::<[u8], ()>(.., |k, v| {
            let g = sh.number(k).filter(|&g| sh.key(g) == k);
            match g.map(|g| (g, check_value(g, v))) {
                Some((g, Ok(()))) if sh.owner(g) == threads => hot_seen += 1,
                Some((_, Ok(()))) => {
                    got.insert(k.to_vec(), v.to_vec());
                }
                Some((_, Err(e))) => bad = Some(e),
                None => bad = Some(format!("a key no thread writes: {k:?}")),
            }
            if bad.is_some() { ControlFlow::Break(()) } else { ControlFlow::Continue(()) }
        });
        if let Some(e) = bad {
            return Err(fail("storelin__torn__final", format!("after the threads: {e}")));
        }
        ensure(hot_seen == sh.hots, "storelin__missing__final", || format!("{hot_seen} of {} hot keys after the threads", sh.hots))?;
        if got != all {
            let missing: Vec<_> = all.iter().filter(|(k, v)| got.get(*k) != Some(*v)).map(|(k, v)| (sh.number(k), short(Some(v)), short(got.get(k).map(|v| &v[..])))).take(4).collect();
            let extra: Vec<_> = got.keys().filter(|k| !all.contains_key(*k)).map(|k| sh.number(k)).take(4).collect();
            return Err(fail("storelin__mismatch__final", format!("after the threads: {} entries, the models {}; differ (number, model, store) {missing:?}, not in the models {extra:?}", got.len(), all.len())));
        }
    }
    classify(refused_total > 0, "refused");
    let out = alloc.0.load(Ordering::Relaxed);
    ensure(out == 0, "storelin__leak__drop", || format!("{out} pages still out after the store was dropped"))
}
