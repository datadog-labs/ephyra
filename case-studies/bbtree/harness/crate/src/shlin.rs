// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! `bbtree::BBTreeMap` with 2 to 6 threads that all write, remove and read
//! the same keys, checked per key for linearizability after the join. No
//! other property lets two threads change one key both ways: synclin's and
//! storelin's keys have one writer, their hot keys are never removed. Here
//! inserts, removes and remove runs, remove_if, update, update_or_insert,
//! get_or_insert and the batch lanes of every thread meet on a small set of
//! hot keys spread over the tree, or on a whole tiny tree that empties and
//! fills (root lowering, the removal fast path), while leaves split and
//! merge under all of them.
//!
//! Every call is stamped with a SeqCst clock before and after. Each written
//! value carries a token unique to the write, so every answer names the one
//! write it saw. Linearizability is local: a history of many keys is
//! linearizable iff each key's is, so each key's calls are checked alone
//! against a one-value register (Wing & Gong search, memoised on the
//! threads' positions and the value). The docs this holds to:
//! - single calls behave as their sequential definitions ("as std's
//!   `BTreeMap::insert` does", `remove`, `remove_if`, `update`,
//!   `update_or_insert`, `get_or_insert`);
//! - batches: lanes "as separate calls to [`Self::insert`] would" in input
//!   order (write batches; "not atomic"), reads "as separate calls to
//!   [`Self::read`] would" (lanes in any order: only the first lane of a
//!   key read is checked);
//! - `scan` "runs `f` on every entry ... each while no thread changes it";
//!   "an entry present for the whole scan is visited exactly once":
//!   per key, a scan's visit is a read at some point of the scan, and a key
//!   in its range not visited was absent at some point of it.
//!   `range_cloned` "hands out an entry other threads insert ahead of it,
//!   and no entry they removed before it got there": the same, per key.
//! Keys between the hot ones (stride > 1) are preloaded and never written:
//! every walk over them must hand them out unchanged. After the join the
//! contents and `check_invariants`.
//! Nondeterministic: repro tries several times.

use std::collections::{BTreeMap, HashSet};
use std::fmt::Debug;
use std::ops::{Bound, ControlFlow};
use std::sync::Barrier;
use std::sync::atomic::{AtomicU64, Ordering};

use std::cell::Cell;

use bbtree::{Allocator, BBTreeMap, InlineKeyStrategy};
use bbtree_store::BBTreeStore;
use bbtree_store::bbtree::{Global, NodeConfig};
use ephyra::{BoxGen, GenExt, classify, gens};
use serde::{Deserialize, Serialize};

use crate::common::{Fail, fail, selftest, spawn_in};
use crate::synclin::{Flaky, Refuse};
use crate::syncthreads::Key;

/// A value of `N` words: key number, token, and `N - 2` words derived from
/// both, so a torn copy or another key's value shows.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct V<const N: usize>([u64; N]);

fn mix(n: u64, p: u64, i: usize) -> u64 {
    let mut x = n.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ p.rotate_left(23) ^ (i as u64).wrapping_mul(0xD6E8_FEB8_6659_FD93);
    x ^= x >> 31;
    x.wrapping_mul(0x94D0_49BB_1331_11EB)
}

impl<const N: usize> V<N> {
    fn new(n: u32, token: u64) -> Self {
        let mut w = [0u64; N];
        w[0] = n as u64;
        w[1] = token;
        for (i, x) in w.iter_mut().enumerate().skip(2) {
            *x = mix(n as u64, token, i);
        }
        V(w)
    }
    fn token(&self) -> u64 {
        self.0[1]
    }
    fn check(&self, n: u32) -> Result<u64, String> {
        if self.0[0] != n as u64 {
            return Err(format!("key number {n} holds a value of key number {}: {self:?}", self.0[0]));
        }
        if (2..N).any(|i| self.0[i] != mix(self.0[0], self.0[1], i)) {
            return Err(format!("key number {n}: torn value {self:?}"));
        }
        Ok(self.0[1])
    }
}

impl<const N: usize> Debug for V<N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "V(n={}, tok={:#x}{})", self.0[0], self.0[1], if (2..N).all(|i| self.0[i] == mix(self.0[0], self.0[1], i)) { "" } else { ", TORN" })
    }
}

/// A write's token: thread, op index and lane, so every token is unique.
fn token(t: usize, step: usize, lane: usize) -> u64 {
    (t as u64 + 1) << 40 | (step as u64) << 12 | lane as u64
}
const PRELOAD: u64 = 0xFF << 40;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Read {
    Copied,
    Cloned,
    Read,
    Contains,
}

/// Key operands are hot-key indices (taken modulo the universe).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Op {
    Insert(u16),
    Remove(u16),
    RemoveEntry(u16),
    /// Removes indices `a, a+1, ...` one call each.
    RemoveRun(u16, u16),
    /// Inserts indices `a, a+1, ...` one call each.
    InsertRun(u16, u16),
    /// remove_if the stored token is odd (true) or even.
    RemoveIf(u16, bool),
    Update(u16),
    Upsert(u16),
    GetOrInsert(u16),
    Get(u16, Read),
    /// insert_many (false) or update_or_insert_many (true); keys may repeat.
    Batch(Vec<u16>, bool),
    GetMany(Vec<u16>, Read),
    /// Scan indices `a..=b`, forward or reverse, breaking after `stop`
    /// entries (0: never).
    Scan(u16, u16, bool, u16),
    /// range_cloned over `a..=b` from the front or the back, `stop` entries.
    Range(u16, u16, bool, u16),
}

/// Hot keys, stride of their key numbers (keys between them preloaded and
/// never written), preload percent, one plan per thread.
pub type Case = (u16, u8, u8, Vec<Vec<Op>>);

fn key_index(universe: u16) -> BoxGen<u16> {
    let u = universe.max(1);
    let hot = u.min(8);
    gens::frequency(vec![
        (3, gens::int(0u16..u).boxed()),
        // a few keys far apart: different leaves, each fought over
        (3, gens::int(0u16..hot).map(move |h| h * (u / hot)).boxed()),
        // the low end (outside the shared prefix under shrink)
        (1, gens::int(0u16..hot).boxed()),
    ])
    .boxed()
}

fn op(universe: u16) -> BoxGen<Op> {
    let i = key_index(universe);
    let read = gens::frequency(vec![
        (3, gens::constant(Read::Copied).boxed()),
        (1, gens::constant(Read::Cloned).boxed()),
        (1, gens::constant(Read::Read).boxed()),
        (1, gens::constant(Read::Contains).boxed()),
    ])
    .boxed();
    let lanes = |g: BoxGen<u16>| {
        gens::frequency(vec![
            (1, gens::vec(g.clone()).len(1..=1).boxed()),
            (4, gens::vec(g.clone()).len(4..=4).boxed()),
            (2, gens::vec(g.clone()).len(16..=16).boxed()),
            (1, gens::vec(g).len(70..=70).boxed()),
        ])
        .boxed()
    };
    let any = gens::int(0u16..universe.max(1)).boxed();
    let stop = gens::frequency(vec![(3, gens::constant(0u16).boxed()), (1, gens::int(1u16..=40).boxed())]).boxed();
    let run = gens::frequency(vec![(3, gens::int(1u16..=40).boxed()), (1, gens::int(40u16..=400).boxed())]).boxed();
    gens::frequency(vec![
        (5, i.clone().map(Op::Insert).boxed()),
        (5, i.clone().map(Op::Remove).boxed()),
        (1, i.clone().map(Op::RemoveEntry).boxed()),
        (1, (i.clone(), run.clone()).map(|(a, n)| Op::RemoveRun(a, n)).boxed()),
        (1, (i.clone(), run).map(|(a, n)| Op::InsertRun(a, n)).boxed()),
        (2, (i.clone(), gens::bool()).map(|(a, b)| Op::RemoveIf(a, b)).boxed()),
        (2, i.clone().map(Op::Update).boxed()),
        (2, i.clone().map(Op::Upsert).boxed()),
        (2, i.clone().map(Op::GetOrInsert).boxed()),
        (5, (i.clone(), read.clone()).map(|(a, r)| Op::Get(a, r)).boxed()),
        (2, (lanes(i.clone()), gens::bool()).map(|(b, u)| Op::Batch(b, u)).boxed()),
        (2, (lanes(i), read).map(|(b, r)| Op::GetMany(b, r)).boxed()),
        (1, (any.clone(), any.clone(), gens::bool(), stop.clone()).map(|(a, b, r, s)| Op::Scan(a.min(b), a.max(b), r, s)).boxed()),
        (1, (any.clone(), any, gens::bool(), stop).map(|(a, b, r, s)| Op::Range(a.min(b), a.max(b), r, s)).boxed()),
    ])
    .boxed()
}

pub fn case() -> BoxGen<Case> {
    let universe = gens::frequency(vec![(1, gens::int(1u16..=4).boxed()), (2, gens::int(5u16..=64).boxed()), (2, gens::int(65u16..=3000).boxed())]);
    gens::dependent(universe, |&u| {
        (
            gens::frequency(vec![(3, gens::constant(1u8).boxed()), (1, gens::int(2u8..=4).boxed())]),
            gens::frequency(vec![(1, gens::constant(0u8).boxed()), (2, gens::int(0u8..=100).boxed())]),
            gens::vec(gens::vec(op(u)).len(0..=800)).len(2..=6),
        )
    })
    .map(|(u, (stride, preload, plans))| (u, stride, preload, plans))
    .boxed()
}

/// What a call did to or saw of one key, by tokens (`None`: absent).
#[derive(Clone, Copy, Debug)]
enum Kind {
    /// insert / update_or_insert / a batch lane: replaced `old`.
    Put { tok: u64, old: Option<u64> },
    Remove { old: Option<u64> },
    RemoveIf { odd: bool, old: Option<u64> },
    Update { tok: u64, old: Option<u64> },
    GetOrInsert { tok: u64, saw: u64 },
    Read { saw: Option<u64> },
    Contains { b: bool },
    /// The store's insert: "returns whether it replaced a value".
    PutB { tok: u64, replaced: bool },
    /// The store's remove: "returns whether the store held it".
    RemoveB { held: bool },
}

impl Kind {
    /// The value after the call from `s`, if the call's answer fits `s`.
    fn apply(self, s: Option<u64>) -> Option<Option<u64>> {
        match self {
            Kind::Put { tok, old } => (s == old).then_some(Some(tok)),
            Kind::Remove { old } => (s == old).then_some(None),
            Kind::RemoveIf { odd, old } => {
                if s.is_some_and(|v| (v & 1 == 1) == odd) {
                    (old == s).then_some(None)
                } else {
                    old.is_none().then_some(s)
                }
            }
            Kind::Update { tok, old } => (s == old).then_some(s.map(|_| tok)),
            Kind::GetOrInsert { tok, saw } => match s {
                Some(v) => (v == saw).then_some(s),
                None => (saw == tok).then_some(Some(tok)),
            },
            Kind::Read { saw } => (s == saw).then_some(s),
            Kind::Contains { b } => (s.is_some() == b).then_some(s),
            Kind::PutB { tok, replaced } => (s.is_some() == replaced).then_some(Some(tok)),
            Kind::RemoveB { held } => (s.is_some() == held).then_some(None),
        }
    }
    fn is_read(self) -> bool {
        matches!(self, Kind::Read { .. } | Kind::Contains { .. })
    }
}

#[derive(Clone, Copy, Debug)]
struct Rec {
    key: u32,
    began: u64,
    ended: u64,
    kind: Kind,
    /// The API call (the read class names a failure).
    op: &'static str,
    batch: bool,
}

/// Whether one key's history is linearizable: `chains` are each thread's
/// calls in program order, `init` / `fin` the value before and after.
/// `None` when the search outgrew `budget` states.
fn linearizable(chains: &[Vec<Rec>], init: Option<u64>, fin: Option<u64>, budget: usize) -> Option<bool> {
    let n = chains.len();
    let mut seen: HashSet<(Vec<u32>, Option<u64>)> = HashSet::new();
    // (positions, value, next chain to try)
    let mut stack: Vec<(Vec<u32>, Option<u64>, usize)> = vec![(vec![0; n], init, 0)];
    seen.insert((vec![0; n], init));
    while let Some((pos, state, next)) = stack.last_mut() {
        if (0..n).all(|c| pos[c] as usize == chains[c].len()) {
            if *state == fin {
                return Some(true);
            }
            stack.pop();
            continue;
        }
        // No call may go before one that ended before it began.
        let min_end = (0..n).filter_map(|c| chains[c].get(pos[c] as usize)).map(|r| r.ended).min().unwrap_or(u64::MAX);
        let mut pushed = None;
        while *next < n {
            let c = *next;
            *next += 1;
            let Some(r) = chains[c].get(pos[c] as usize) else { continue };
            if r.began > min_end {
                continue;
            }
            if let Some(s2) = r.kind.apply(*state) {
                let mut p2 = pos.clone();
                p2[c] += 1;
                if seen.insert((p2.clone(), s2)) {
                    pushed = Some((p2, s2));
                    break;
                }
            }
        }
        match pushed {
            Some((p2, s2)) => {
                if seen.len() > budget {
                    return None;
                }
                stack.push((p2, s2, 0));
            }
            None => {
                stack.pop();
            }
        }
    }
    Some(false)
}

/// Which calls a non-linearizable key history blames: the read class whose
/// removal makes it linearizable (removing reads never breaks a history),
/// else the writes (with or without batch lanes among them).
fn blame(chains: &[Vec<Rec>], init: Option<u64>, fin: Option<u64>) -> &'static str {
    let mut classes: Vec<&'static str> = chains.iter().flatten().filter(|r| r.kind.is_read()).map(|r| r.op).collect();
    classes.sort();
    classes.dedup();
    for class in classes {
        let kept: Vec<Vec<Rec>> = chains.iter().map(|ch| ch.iter().filter(|r| r.op != class).copied().collect()).collect();
        if linearizable(&kept, init, fin, 400_000) == Some(true) {
            return class;
        }
    }
    let writes: Vec<Vec<Rec>> = chains.iter().map(|ch| ch.iter().filter(|r| !r.kind.is_read()).copied().collect()).collect();
    if linearizable(&writes, init, fin, 400_000) == Some(true) {
        return "reads";
    }
    if chains.iter().flatten().any(|r| r.batch) { "writes_batch" } else { "writes" }
}

fn sig(kind: &str, area: &str) -> String {
    format!("shlin__{kind}__{area}")
}

struct Shape {
    universe: u32,
    stride: u32,
}

impl Shape {
    fn number(&self, i: u32) -> u32 {
        i * self.stride
    }
    fn hot(&self, n: u32) -> Option<u32> {
        (n % self.stride == 0 && n / self.stride < self.universe).then_some(n / self.stride)
    }
    fn preloaded(&self, n: u32, preload: u32) -> bool {
        (n.wrapping_mul(2_654_435_761) >> 7) % 100 < preload
    }
}

/// The lanes' results, a refused lane's entry as it came back; `ran`
/// counts the closure's runs.
#[allow(clippy::type_complexity)]
fn batch<K: Key, const N: usize, S: InlineKeyStrategy<K>, A: Allocator, const B: usize>(map: &BBTreeMap<K, V<N>, S, A>, entries: Vec<(K, V<N>)>, upsert: bool, ran: &Cell<usize>) -> Vec<Result<Option<V<N>>, (K, V<N>)>> {
    let mut it = entries.into_iter();
    let arr: [(K, V<N>); B] = std::array::from_fn(|_| it.next().unwrap());
    if upsert {
        map.update_or_insert_many(arr, |_, s, new| {
            ran.set(ran.get() + 1);
            std::mem::replace(s, new)
        })
        .into_iter()
        .map(|r| r.map_err(|e| e.into_entry()))
        .collect()
    } else {
        map.insert_many(arr).into_iter().map(|r| r.map_err(|e| e.into_entry())).collect()
    }
}

fn get_many<K: Key, const N: usize, S: InlineKeyStrategy<K>, A: Allocator, const B: usize>(map: &BBTreeMap<K, V<N>, S, A>, keys: &[K], how: Read) -> Vec<Option<V<N>>> {
    let refs: [&K; B] = std::array::from_fn(|j| &keys[j]);
    match how {
        Read::Copied => map.get_many_copied(refs).to_vec(),
        Read::Cloned => map.get_many_cloned(refs).to_vec(),
        _ => map.read_many(refs, |_, v| *v).to_vec(),
    }
}

fn lanes(len: usize) -> usize {
    match len {
        0 => 0,
        1..4 => 1,
        4..16 => 4,
        16..70 => 16,
        _ => 70,
    }
}

/// Each key's history alone: linearizable from `init` to `fin`.
fn judge(sh: &Shape, init: &[Option<u64>], fin: &[Option<u64>], all: &[Vec<Rec>]) -> Result<(), Fail> {
    // Per key, each thread's calls in program order.
    let mut per_key: Vec<Vec<Vec<Rec>>> = vec![vec![Vec::new(); all.len()]; sh.universe as usize];
    for (t, recs) in all.iter().enumerate() {
        for r in recs {
            per_key[r.key as usize][t].push(*r);
        }
    }
    let mut gave_up = false;
    let mut contested = false;
    for (i, chains) in per_key.iter().enumerate() {
        let writers = chains.iter().filter(|ch| ch.iter().any(|r| !r.kind.is_read())).count();
        contested |= writers >= 2;
        match linearizable(chains, init[i], fin[i], 200_000) {
            Some(true) => {}
            None => gave_up = true,
            Some(false) => {
                let area = blame(chains, init[i], fin[i]);
                let calls: usize = chains.iter().map(Vec::len).sum();
                let mut shown: Vec<(usize, Rec)> = chains.iter().enumerate().flat_map(|(t, ch)| ch.iter().map(move |r| (t, *r))).collect();
                shown.sort_by_key(|(_, r)| r.began);
                let lines: Vec<String> = shown.iter().take(60).map(|(t, r)| format!("  t{t} [{}, {}] {} {:?}", r.began, r.ended, r.op, r.kind)).collect();
                return Err(fail(
                    &sig("nonlinear", area),
                    format!("key number {} (hot index {i}): no sequential order of its {calls} calls explains their answers (before {:?}, after {:?}); blamed: {area}; first calls:\n{}", sh.number(i as u32), init[i], fin[i], lines.join("\n")),
                ));
            }
        }
    }
    classify(gave_up, "lin_giveup");
    classify(contested, "contested");
    classify(sh.universe <= 4, "tiny_tree");
    Ok(())
}

/// A walk over key numbers `lo..=hi` that handed out `seen` (ascending,
/// tokens), and stopped early if `stopped`: it covered the range up to the
/// last entry it handed out. Every never-written key it covered must be in
/// `seen` unchanged; every hot key it covered is a read of that key at some
/// point of the walk (`None` if not handed out).
#[allow(clippy::too_many_arguments)]
fn walked(sh: &Shape, steady: &BTreeMap<u32, u64>, (lo, hi): (u32, u32), stopped: bool, rev: bool, seen: &[(u32, u64)], (name, b, e): (&'static str, u64, u64), recs: &mut Vec<Rec>) -> Result<(), (&'static str, String)> {
    let (clo, chi) = match (stopped, rev, seen.first(), seen.last()) {
        (true, false, _, Some(l)) => (lo, l.0),
        (true, true, Some(f), _) => (f.0, hi),
        _ => (lo, hi),
    };
    let visited: BTreeMap<u32, u64> = seen.iter().copied().collect();
    for (&n, v) in steady.range(clo..=chi) {
        if visited.get(&n) != Some(v) {
            return Err(("missing", format!("key number {n} (never written, present all along, token {v:#x}) handed out as {:?}; walk {lo}..={hi} covered {clo}..={chi}", visited.get(&n))));
        }
    }
    for i in clo.div_ceil(sh.stride)..=chi / sh.stride {
        if i >= sh.universe {
            break;
        }
        let saw = visited.get(&sh.number(i)).copied();
        recs.push(Rec { key: i, began: b, ended: e, kind: Kind::Read { saw }, op: name, batch: false });
    }
    Ok(())
}

pub fn run<K: Key, S: InlineKeyStrategy<K> + Default + Send + Sync, const N: usize, const SHRINK: bool>(case: &Case) -> Result<(), Fail>
where
    BBTreeMap<K, V<N>, S>: Sync,
{
    run_in::<K, S, Global, N, SHRINK>(case, Global, &[], &|| 0)
}

/// A shlin case, and how each thread refuses pages (thread `t` by entry `t % len`).
pub type RCase = (Case, Vec<Refuse>);

pub fn rcase() -> BoxGen<RCase> {
    (case(), crate::synclin::refuses()).boxed()
}

/// Refused writes of the case running (a label, counted on the threads).
static REFUSED: AtomicU64 = AtomicU64::new(0);

/// A refused call's name in the history: a read that the key was absent
/// ("If the allocator has no page for a node the insert makes, the map is
/// unchanged": an insert of a key the map holds makes no node).
fn refused_name(area: &str) -> &'static str {
    match area {
        "insert" => "refused_insert",
        "upsert" => "refused_upsert",
        "get_or_insert" => "refused_get_or_insert",
        "insert_many" => "refused_insert_many",
        _ => "refused_update_or_insert_many",
    }
}

/// shlin on a [`Flaky`] allocator that refuses pages to thread `t` as
/// `refuse[t % len]` says: a refused write hands its entry back, runs no
/// closure, and is in the key's history as a read that found it absent.
/// Every page back after the drop. Signatures `shlinr__<kind>__<area>`.
pub fn run_refuse<K: Key, S: InlineKeyStrategy<K> + Send + Sync, const N: usize, const SHRINK: bool>(case: &RCase) -> Result<(), Fail>
where
    BBTreeMap<K, V<N>, S, Flaky>: Sync,
{
    let (case, refuse) = case;
    let alloc = Flaky::default();
    let pages = alloc.clone();
    let refuse = if refuse.is_empty() { vec![Refuse::NONE] } else { refuse.clone() };
    REFUSED.store(0, Ordering::Relaxed);
    let r = run_in::<K, S, Flaky, N, SHRINK>(case, alloc, &refuse, &|| pages.out()).map_err(|mut f| {
        if let Some(rest) = f.sig.strip_prefix("shlin__") {
            f.sig = format!("shlinr__{rest}");
        }
        f
    });
    classify(REFUSED.load(Ordering::Relaxed) > 0, "refused");
    r
}

fn run_in<K: Key, S: InlineKeyStrategy<K> + Send + Sync, A: Allocator + Send + Sync, const N: usize, const SHRINK: bool>(
    case: &Case,
    alloc: A,
    refuse: &[Refuse],
    pages_out: &dyn Fn() -> isize,
) -> Result<(), Fail>
where
    BBTreeMap<K, V<N>, S, A>: Sync,
{
    let lenient = !refuse.is_empty();
    let (universe, stride, preload, plans) = case;
    let shape = Shape { universe: (*universe).max(1) as u32, stride: (*stride).clamp(1, 4) as u32 };
    let preload = *preload as u32;
    let sh = &shape;
    #[allow(unused_mut)]
    let mut map = BBTreeMap::<K, V<N>, S, A>::new_in(alloc);
    let key = |n: u32| K::of(n, SHRINK);
    let top = shape.number(shape.universe);
    // Preload: hot keys (their first value) and the never-written keys
    // between them; under shrink none outside the shared prefix, so the
    // first thread to insert one shrinks it while the others run.
    let mut init: Vec<Option<u64>> = vec![None; shape.universe as usize];
    let mut steady: BTreeMap<u32, V<N>> = BTreeMap::new();
    for n in 0..top {
        if !shape.preloaded(n, preload) || (SHRINK && K::outside(n)) {
            continue;
        }
        let v = V::new(n, PRELOAD | n as u64);
        map.insert(key(n), v).map_err(|_| fail("shlin__refused__preload", format!("preload {n} refused")))?;
        match shape.hot(n) {
            Some(i) => init[i as usize] = Some(v.token()),
            None => {
                steady.insert(n, v);
            }
        }
    }
    let steady_tok: &BTreeMap<u32, u64> = &steady.iter().map(|(&n, v)| (n, v.token())).collect();
    let steady = &steady;
    let init_ref = &init;
    let clock = AtomicU64::new(0);
    let start = Barrier::new(plans.len());
    let results: Vec<Result<Vec<Rec>, Fail>> = std::thread::scope(|scope| {
        let handles: Vec<_> = plans
            .iter()
            .enumerate()
            .map(|(t, plan)| {
                let (map, start, clock) = (&map, &start, &clock);
                spawn_in(scope, move || -> Result<Vec<Rec>, Fail> {
                    let mut recs: Vec<Rec> = Vec::new();
                    // BBZ_SELFTEST_BREAK: get_copied of a key this thread
                    // removed reports the key's first value (a stale read).
                    let broken = selftest("break");
                    let idx = |i: u16| i as u32 % sh.universe;
                    let tick = || clock.fetch_add(1, Ordering::SeqCst);
                    let refusing = refuse.get(t % refuse.len().max(1)).copied();
                    let _reset = Flaky::arm(refusing.map_or(0, |r| r.seed(t)));
                    start.wait();
                    for (step, op) in plan.iter().enumerate() {
                        let ctx = |kind: &str, area: &str, why: String| fail(&sig(kind, area), format!("thread {t} op {step} {op:?}: {why}"));
                        if let Some(r) = refusing {
                            Flaky::set_rate(r.rate_at(step));
                        }
                        // A refused write: its entry back as given, no
                        // closure run; the key was absent when it was refused.
                        let refused = |(rk, rv): (K, V<N>), n: u32, v: V<N>, ran: bool, area: &str| -> Result<(Kind, &'static str), Fail> {
                            if !lenient {
                                return Err(ctx("refused", area, "refused".into()));
                            }
                            REFUSED.fetch_add(1, Ordering::Relaxed);
                            if rk != key(n) || rv != v {
                                return Err(ctx("refused_entry", area, format!("refused entry came back as ({rk:?}, {:?}), given key number {n}", rv.token())));
                            }
                            if ran {
                                return Err(ctx("closure_ran", area, format!("the closure ran, yet the write of key number {n} was refused")));
                            }
                            Ok((Kind::Contains { b: false }, refused_name(area)))
                        };
                        // A value handed out for key number n: whole, its own.
                        let tok = |n: u32, v: &V<N>, area: &str| v.check(n).map_err(|why| ctx("torn", area, why));
                        let otok = |n: u32, v: Option<V<N>>, area: &str| v.map(|v| tok(n, &v, area)).transpose();
                        macro_rules! rec {
                            ($i:expr, $b:expr, $e:expr, $kind:expr, $op:expr, $batch:expr) => {
                                recs.push(Rec { key: $i, began: $b, ended: $e, kind: $kind, op: $op, batch: $batch })
                            };
                        }
                        // One single-key write call on hot index i.
                        let single = |i: u32, which: &'static str, lane: usize, recs: &mut Vec<Rec>| -> Result<(), Fail> {
                            let n = sh.number(i);
                            let tk = token(t, step, lane);
                            let v = V::new(n, tk);
                            let b = tick();
                            let mut name = which;
                            let kind = match which {
                                "insert" => match map.insert(key(n), v) {
                                    Ok(old) => Kind::Put { tok: tk, old: otok(n, old, "insert")? },
                                    Err(e) => {
                                        let (kind, r) = refused(e.into_entry(), n, v, false, "insert")?;
                                        name = r;
                                        kind
                                    }
                                },
                                "remove" => Kind::Remove { old: otok(n, map.remove(&key(n)), "remove")? },
                                "remove_entry" => {
                                    let r = map.remove_entry(&key(n));
                                    if let Some((k, _)) = &r
                                        && *k != key(n)
                                    {
                                        return Err(ctx("mismatch", "remove_entry", format!("removed key {k:?} for {:?}", key(n))));
                                    }
                                    Kind::Remove { old: otok(n, r.map(|(_, v)| v), "remove_entry")? }
                                }
                                _ => unreachable!(),
                            };
                            let e = tick();
                            recs.push(Rec { key: i, began: b, ended: e, kind, op: name, batch: false });
                            Ok(())
                        };
                        match op {
                            Op::Insert(i) => single(idx(*i), "insert", 0, &mut recs)?,
                            Op::Remove(i) => single(idx(*i), "remove", 0, &mut recs)?,
                            Op::RemoveEntry(i) => single(idx(*i), "remove_entry", 0, &mut recs)?,
                            Op::RemoveRun(a, len) => {
                                for j in 0..(*len as u32).min(sh.universe) {
                                    single((idx(*a) + j) % sh.universe, "remove", j as usize, &mut recs)?;
                                }
                            }
                            Op::InsertRun(a, len) => {
                                for j in 0..(*len as u32).min(sh.universe) {
                                    single((idx(*a) + j) % sh.universe, "insert", j as usize, &mut recs)?;
                                }
                            }
                            Op::RemoveIf(i, odd) => {
                                let (i, odd) = (idx(*i), *odd);
                                let n = sh.number(i);
                                let b = tick();
                                let r = map.remove_if(&key(n), |_, v| (v.token() & 1 == 1) == odd).map(|(_, v)| v);
                                let e = tick();
                                rec!(i, b, e, Kind::RemoveIf { odd, old: otok(n, r, "remove_if")? }, "remove_if", false);
                            }
                            Op::Update(i) => {
                                let i = idx(*i);
                                let n = sh.number(i);
                                let tk = token(t, step, 0);
                                let v = V::new(n, tk);
                                let b = tick();
                                let r = map.update(&key(n), |_, s| std::mem::replace(s, v));
                                let e = tick();
                                rec!(i, b, e, Kind::Update { tok: tk, old: otok(n, r, "update")? }, "update", false);
                            }
                            Op::Upsert(i) => {
                                let i = idx(*i);
                                let n = sh.number(i);
                                let tk = token(t, step, 0);
                                let ran = Cell::new(false);
                                let b = tick();
                                let r = map.update_or_insert(key(n), V::new(n, tk), |_, s, new| {
                                    ran.set(true);
                                    std::mem::replace(s, new)
                                });
                                let e = tick();
                                match r {
                                    Ok(r) => rec!(i, b, e, Kind::Put { tok: tk, old: otok(n, r, "upsert")? }, "upsert", false),
                                    Err(x) => {
                                        let (kind, name) = refused(x.into_entry(), n, V::new(n, tk), ran.get(), "upsert")?;
                                        rec!(i, b, e, kind, name, false);
                                    }
                                }
                            }
                            Op::GetOrInsert(i) => {
                                let i = idx(*i);
                                let n = sh.number(i);
                                let tk = token(t, step, 0);
                                let ran = Cell::new(false);
                                let b = tick();
                                let r = map.get_or_insert(key(n), V::new(n, tk), |_, s| {
                                    ran.set(true);
                                    *s
                                });
                                let e = tick();
                                match r {
                                    Ok(r) => rec!(i, b, e, Kind::GetOrInsert { tok: tk, saw: tok(n, &r, "get_or_insert")? }, "get_or_insert", false),
                                    Err(x) => {
                                        let (kind, name) = refused(x.into_entry(), n, V::new(n, tk), ran.get(), "get_or_insert")?;
                                        rec!(i, b, e, kind, name, false);
                                    }
                                }
                            }
                            Op::Get(i, how) => {
                                let i = idx(*i);
                                let n = sh.number(i);
                                let k = key(n);
                                let b = tick();
                                let (kind, name) = match how {
                                    Read::Copied => {
                                        let mut saw = otok(n, map.get_copied(&k), "get_copied")?;
                                        if broken && init_ref[i as usize].is_some() && recs.iter().any(|r| r.key == i && matches!(r.kind, Kind::Remove { old: Some(_) })) {
                                            saw = init_ref[i as usize];
                                        }
                                        (Kind::Read { saw }, "get_copied")
                                    }
                                    Read::Cloned => (Kind::Read { saw: otok(n, map.get_cloned(&k), "get_cloned")? }, "get_cloned"),
                                    Read::Read => (Kind::Read { saw: otok(n, map.read(&k, |_, v| *v), "read")? }, "read"),
                                    Read::Contains => (Kind::Contains { b: map.contains_key(&k) }, "contains_key"),
                                };
                                let e = tick();
                                rec!(i, b, e, kind, name, false);
                            }
                            Op::Batch(keys, upsert) => {
                                let w = lanes(keys.len());
                                if w == 0 {
                                    continue;
                                }
                                let is: Vec<u32> = keys[..w].iter().map(|&i| idx(i)).collect();
                                let entries: Vec<(K, V<N>)> = is.iter().enumerate().map(|(l, &i)| (key(sh.number(i)), V::new(sh.number(i), token(t, step, l)))).collect();
                                let name = if *upsert { "update_or_insert_many" } else { "insert_many" };
                                let ran = Cell::new(0);
                                let b = tick();
                                let out = match w {
                                    1 => batch::<K, N, S, A, 1>(map, entries, *upsert, &ran),
                                    4 => batch::<K, N, S, A, 4>(map, entries, *upsert, &ran),
                                    16 => batch::<K, N, S, A, 16>(map, entries, *upsert, &ran),
                                    _ => batch::<K, N, S, A, 70>(map, entries, *upsert, &ran),
                                };
                                let e = tick();
                                // Every closure run is a lane that found its key.
                                let found = out.iter().filter(|r| matches!(r, Ok(Some(_)))).count();
                                if *upsert && ran.get() != found {
                                    return Err(ctx("closure_ran", name, format!("the closure ran {} times for {found} lanes that found their key", ran.get())));
                                }
                                // Lanes in input order, as separate calls.
                                for (l, (&i, r)) in is.iter().zip(out).enumerate() {
                                    let n = sh.number(i);
                                    match r {
                                        Ok(r) => rec!(i, b, e, Kind::Put { tok: token(t, step, l), old: otok(n, r, name)? }, name, true),
                                        Err(x) => {
                                            let (kind, rname) = refused(x, n, V::new(n, token(t, step, l)), false, name)?;
                                            rec!(i, b, e, kind, rname, true);
                                        }
                                    }
                                }
                            }
                            Op::GetMany(keys, how) => {
                                let w = lanes(keys.len());
                                if w == 0 {
                                    continue;
                                }
                                let is: Vec<u32> = keys[..w].iter().map(|&i| idx(i)).collect();
                                let ks: Vec<K> = is.iter().map(|&i| key(sh.number(i))).collect();
                                let name = match how {
                                    Read::Copied => "get_many_copied",
                                    Read::Cloned => "get_many_cloned",
                                    _ => "read_many",
                                };
                                let b = tick();
                                let got = match w {
                                    1 => get_many::<K, N, S, A, 1>(map, &ks, *how),
                                    4 => get_many::<K, N, S, A, 4>(map, &ks, *how),
                                    16 => get_many::<K, N, S, A, 16>(map, &ks, *how),
                                    _ => get_many::<K, N, S, A, 70>(map, &ks, *how),
                                };
                                let e = tick();
                                // Lanes of one key read in any order: the first only.
                                let mut done = HashSet::new();
                                for (&i, v) in is.iter().zip(got) {
                                    let saw = otok(sh.number(i), v, name)?;
                                    if done.insert(i) {
                                        rec!(i, b, e, Kind::Read { saw }, name, true);
                                    }
                                }
                            }
                            Op::Scan(a, z, rev, stop) | Op::Range(a, z, rev, stop) => {
                                let is_scan = matches!(op, Op::Scan(..));
                                let name: &'static str = match (is_scan, *rev) {
                                    (true, false) => "scan",
                                    (true, true) => "scan_rev",
                                    (false, false) => "range_cloned",
                                    (false, true) => "range_cloned_rev",
                                };
                                let (lo, hi) = (sh.number(idx(*a).min(idx(*z))), sh.number(idx(*a).max(idx(*z))));
                                let (klo, khi) = (key(lo), key(hi));
                                let stop = *stop as usize;
                                let mut seen: Vec<(K, V<N>)> = Vec::new();
                                let b = tick();
                                if is_scan {
                                    let visit = |k: &K, v: &V<N>| {
                                        seen.push((k.clone(), *v));
                                        if stop != 0 && seen.len() >= stop { ControlFlow::Break(()) } else { ControlFlow::Continue(()) }
                                    };
                                    let range = (Bound::Included(klo.clone()), Bound::Included(khi.clone()));
                                    if *rev { map.scan_rev(range, visit) } else { map.scan(range, visit) };
                                } else {
                                    let mut it = map.range_cloned(klo.clone()..=khi.clone());
                                    while stop == 0 || seen.len() < stop {
                                        match if *rev { it.next_back() } else { it.next() } {
                                            Some(e) => seen.push(e),
                                            None => break,
                                        }
                                    }
                                }
                                let e = tick();
                                if *rev {
                                    seen.reverse();
                                }
                                let mut visited: BTreeMap<u32, V<N>> = BTreeMap::new();
                                for (j, (k, v)) in seen.iter().enumerate() {
                                    if j > 0 && *k <= seen[j - 1].0 || *k < klo || *k > khi {
                                        return Err(ctx("order", name, format!("handed {k:?} after {:?} (in key order), range {klo:?}..={khi:?}", j.checked_sub(1).map(|p| &seen[p].0))));
                                    }
                                    let n = k.number(SHRINK);
                                    if key(n) != *k || (sh.hot(n).is_none() && !steady.contains_key(&n)) {
                                        return Err(ctx("phantom", name, format!("handed {k:?} = {v:?}, a key no thread ever wrote")));
                                    }
                                    tok(n, v, name)?;
                                    visited.insert(n, *v);
                                }
                                let seen: Vec<(u32, u64)> = visited.iter().map(|(&n, v)| (n, v.token())).collect();
                                walked(sh, steady_tok, (lo, hi), stop != 0 && seen.len() >= stop, *rev, &seen, (name, b, e), &mut recs).map_err(|(kind, why)| ctx(kind, name, why))?;
                            }
                        }
                    }
                    Ok(recs)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| {
                h.join().unwrap_or_else(|p| {
                    let msg = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()));
                    Err(fail("shlin__panic__thread", format!("thread panicked: {msg:?}")))
                })
            })
            .collect()
    });
    let mut all = Vec::new();
    for r in results {
        all.push(r?);
    }
    // After the join: the steady keys unchanged, hot keys' last values.
    let mut fin: Vec<Option<u64>> = vec![None; shape.universe as usize];
    let mut count = 0usize;
    let mut bad: Option<Fail> = None;
    map.scan::<K, ()>(.., |k, v| {
        count += 1;
        let n = k.number(SHRINK);
        let r = match (shape.hot(n), steady.get(&n)) {
            (Some(i), _) => v.check(n).map(|tk| fin[i as usize] = Some(tk)),
            (None, Some(w)) if w == v => Ok(()),
            (None, w) => Err(format!("key {k:?} holds {v:?}, want {w:?} (never written)")),
        };
        match r {
            Ok(()) => ControlFlow::Continue(()),
            Err(why) => {
                bad = Some(fail("shlin__final__contents", format!("after the join: {why}")));
                ControlFlow::Break(())
            }
        }
    });
    if let Some(f) = bad {
        return Err(f);
    }
    let held = fin.iter().filter(|v| v.is_some()).count();
    if count != held + steady.len() {
        return Err(fail("shlin__final__contents", format!("after the join: {count} entries, {held} hot + {} never written expected", steady.len())));
    }
    judge(&shape, &init, &fin, &all)?;
    #[cfg(fuzzing)]
    {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| map.check_invariants(count)));
        if let Err(p) = r {
            let msg = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()));
            return Err(fail("shlin__invariant__check", format!("after the join ({count} entries): {msg:?}")));
        }
    }
    drop(map);
    let out = pages_out();
    if out != 0 {
        return Err(fail("shlin__leak__pages", format!("{out} pages out after the map was dropped")));
    }
    Ok(())
}

/// The store's keys by shape: short; behind a 30-byte shared prefix; 127
/// bytes; and under 16 outside that prefix (the first insert of one
/// shrinks it). Ordered as their numbers.
fn skey(n: u32, ks: u8) -> Vec<u8> {
    match ks {
        0 => format!("k{n:07}"),
        1 => format!("tenant/0042/collection/abcdef/{n:07}"),
        2 => format!("{n:07}{}", "x".repeat(120)),
        _ if n < 16 => format!("t{n:02}"),
        _ => format!("tenant/0042/collection/abcdef/{n:07}"),
    }
    .into_bytes()
}

fn snum(k: &[u8], ks: u8) -> Option<u32> {
    let s = std::str::from_utf8(k).ok()?;
    let n = match ks {
        2 => s.get(..7)?.parse().ok()?,
        3 if s.len() == 3 => s.get(1..)?.parse().ok()?,
        _ => s.get(s.len().checked_sub(7)?..)?.parse().ok()?,
    };
    (skey(n, ks) == k).then_some(n)
}

/// A store value: key number, token, then bytes derived from both; its
/// length from the token (mostly short, some past a heap page: overflow).
fn sval(n: u32, tok: u64) -> Vec<u8> {
    let h = mix(n as u64, tok, 1);
    let len = match h % 10 {
        0..7 => 12 + (h >> 8) % 52,
        7..9 => 100 + (h >> 8) % 800,
        _ => 1500 + (h >> 8) % 18_500,
    } as usize;
    let mut v = Vec::with_capacity(len);
    v.extend_from_slice(&n.to_le_bytes());
    v.extend_from_slice(&tok.to_le_bytes());
    for i in 12..len {
        v.push((mix(n as u64, tok, i / 8 + 2) >> (8 * (i % 8))) as u8);
    }
    v
}

fn scheck(n: u32, v: &[u8]) -> Result<u64, String> {
    if v.len() < 12 {
        return Err(format!("key number {n}: value of {} bytes", v.len()));
    }
    let m = u32::from_le_bytes(v[..4].try_into().unwrap());
    let tok = u64::from_le_bytes(v[4..12].try_into().unwrap());
    if m != n {
        return Err(format!("key number {n} holds a value of key number {m}"));
    }
    if sval(n, tok) != v {
        return Err(format!("key number {n}: torn value ({} bytes, token {tok:#x}, want {} bytes)", v.len(), sval(n, tok).len()));
    }
    Ok(tok)
}

/// The same threads on a `BBTreeStore`: insert ("returns whether it replaced
/// a value"), remove ("whether the store held it"), get, get_into,
/// contains_key, scan / scan_rev / range from both ends. Batches and the
/// map's other writes become runs of inserts and gets.
pub fn run_store<C: NodeConfig + Default + Send + Sync + 'static, const KS: u8>(case: &Case) -> Result<(), Fail>
where
    BBTreeStore<Global, C>: Sync,
{
    let (universe, stride, preload, plans) = case;
    let shape = Shape { universe: (*universe).max(1) as u32, stride: (*stride).clamp(1, 4) as u32 };
    let preload = *preload as u32;
    let sh = &shape;
    let store = BBTreeStore::with_config_in(C::default(), Global);
    let top = shape.number(shape.universe);
    let mut init: Vec<Option<u64>> = vec![None; shape.universe as usize];
    let mut steady: BTreeMap<u32, u64> = BTreeMap::new();
    for n in 0..top {
        if !shape.preloaded(n, preload) || (KS == 3 && n < 16) {
            continue;
        }
        let tk = PRELOAD | n as u64;
        store.insert(&skey(n, KS), &sval(n, tk)).map_err(|e| fail("shlin__refused__preload", format!("preload {n}: {e}")))?;
        match shape.hot(n) {
            Some(i) => init[i as usize] = Some(tk),
            None => {
                steady.insert(n, tk);
            }
        }
    }
    let steady = &steady;
    let clock = AtomicU64::new(0);
    let start = Barrier::new(plans.len());
    let results: Vec<Result<Vec<Rec>, Fail>> = std::thread::scope(|scope| {
        let handles: Vec<_> = plans
            .iter()
            .enumerate()
            .map(|(t, plan)| {
                let (store, start, clock) = (&store, &start, &clock);
                spawn_in(scope, move || -> Result<Vec<Rec>, Fail> {
                    let mut recs: Vec<Rec> = Vec::new();
                    let idx = |i: u16| i as u32 % sh.universe;
                    let tick = || clock.fetch_add(1, Ordering::SeqCst);
                    start.wait();
                    for (step, op) in plan.iter().enumerate() {
                        let ctx = |kind: &str, area: &str, why: String| fail(&sig(kind, area), format!("store thread {t} op {step} {op:?}: {why}"));
                        let tok = |n: u32, v: &[u8], area: &str| scheck(n, v).map_err(|why| ctx("torn", area, why));
                        let put = |i: u32, lane: usize, recs: &mut Vec<Rec>| -> Result<(), Fail> {
                            let n = sh.number(i);
                            let tk = token(t, step, lane);
                            let b = tick();
                            let r = store.insert(&skey(n, KS), &sval(n, tk)).map_err(|e| ctx("refused", "store_insert", format!("{e}")))?;
                            let e = tick();
                            recs.push(Rec { key: i, began: b, ended: e, kind: Kind::PutB { tok: tk, replaced: r }, op: "store_insert", batch: false });
                            Ok(())
                        };
                        let del = |i: u32, recs: &mut Vec<Rec>| {
                            let b = tick();
                            let r = store.remove(&skey(sh.number(i), KS));
                            let e = tick();
                            recs.push(Rec { key: i, began: b, ended: e, kind: Kind::RemoveB { held: r }, op: "store_remove", batch: false });
                        };
                        let get = |i: u32, how: Read, recs: &mut Vec<Rec>| -> Result<(), Fail> {
                            let n = sh.number(i);
                            let k = skey(n, KS);
                            let b = tick();
                            let (kind, name) = match how {
                                Read::Contains => (Kind::Contains { b: store.contains_key(&k) }, "store_contains_key"),
                                Read::Read => {
                                    let junk = vec![0xA5u8; (step % 5) * 7];
                                    let mut buf = junk.clone();
                                    let r = store.get_into(&k, &mut buf);
                                    let saw = match r {
                                        None if buf == junk => None,
                                        Some(len) if buf.len() == junk.len() + len && buf[..junk.len()] == junk[..] => Some(tok(n, &buf[junk.len()..], "store_get_into")?),
                                        _ => return Err(ctx("mismatch", "store_get_into", format!("returned {r:?}, buffer of {} bytes after {} prefilled", buf.len(), junk.len()))),
                                    };
                                    (Kind::Read { saw }, "store_get_into")
                                }
                                _ => (Kind::Read { saw: store.get(&k).map(|v| tok(n, &v, "store_get")).transpose()? }, "store_get"),
                            };
                            let e = tick();
                            recs.push(Rec { key: i, began: b, ended: e, kind, op: name, batch: false });
                            Ok(())
                        };
                        match op {
                            Op::Insert(i) | Op::Update(i) | Op::Upsert(i) => put(idx(*i), 0, &mut recs)?,
                            Op::Remove(i) | Op::RemoveEntry(i) | Op::RemoveIf(i, _) => del(idx(*i), &mut recs),
                            Op::RemoveRun(a, len) => {
                                for j in 0..(*len as u32).min(sh.universe) {
                                    del((idx(*a) + j) % sh.universe, &mut recs);
                                }
                            }
                            Op::InsertRun(a, len) => {
                                for j in 0..(*len as u32).min(sh.universe) {
                                    put((idx(*a) + j) % sh.universe, j as usize, &mut recs)?;
                                }
                            }
                            Op::Batch(keys, _) => {
                                for (l, &i) in keys.iter().take(16).enumerate() {
                                    put(idx(i), l, &mut recs)?;
                                }
                            }
                            Op::GetOrInsert(i) => get(idx(*i), Read::Read, &mut recs)?,
                            Op::Get(i, how) => get(idx(*i), *how, &mut recs)?,
                            Op::GetMany(keys, how) => {
                                for &i in keys.iter().take(16) {
                                    get(idx(i), *how, &mut recs)?;
                                }
                            }
                            Op::Scan(a, z, rev, stop) | Op::Range(a, z, rev, stop) => {
                                let is_scan = matches!(op, Op::Scan(..));
                                let name: &'static str = match (is_scan, *rev) {
                                    (true, false) => "store_scan",
                                    (true, true) => "store_scan_rev",
                                    (false, false) => "store_range",
                                    (false, true) => "store_range_rev",
                                };
                                let (lo, hi) = (sh.number(idx(*a).min(idx(*z))), sh.number(idx(*a).max(idx(*z))));
                                let (klo, khi) = (skey(lo, KS), skey(hi, KS));
                                let stop = *stop as usize;
                                let mut seen: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
                                let b = tick();
                                if is_scan {
                                    let visit = |k: &[u8], v: &[u8]| {
                                        seen.push((k.to_vec(), v.to_vec()));
                                        if stop != 0 && seen.len() >= stop { ControlFlow::Break(()) } else { ControlFlow::Continue(()) }
                                    };
                                    let range = (Bound::Included(&klo[..]), Bound::Included(&khi[..]));
                                    if *rev { store.scan_rev::<[u8], ()>(range, visit) } else { store.scan::<[u8], ()>(range, visit) };
                                } else {
                                    let mut it = store.range::<[u8], _>((Bound::Included(&klo[..]), Bound::Included(&khi[..])));
                                    while stop == 0 || seen.len() < stop {
                                        match if *rev { it.next_back() } else { it.next() } {
                                            Some(e) => seen.push(e),
                                            None => break,
                                        }
                                        if seen.len() > 200_000 {
                                            return Err(ctx("runaway", name, "handed out more than 200000 entries".into()));
                                        }
                                    }
                                }
                                let e = tick();
                                if *rev {
                                    seen.reverse();
                                }
                                let mut toks: Vec<(u32, u64)> = Vec::new();
                                for (j, (k, v)) in seen.iter().enumerate() {
                                    if j > 0 && *k <= seen[j - 1].0 || *k < klo || *k > khi {
                                        return Err(ctx("order", name, format!("handed {:?} after {:?} (in key order), range {lo}..={hi}", String::from_utf8_lossy(k), j.checked_sub(1).map(|p| String::from_utf8_lossy(&seen[p].0)))));
                                    }
                                    let Some(n) = snum(k, KS).filter(|&n| sh.hot(n).is_some() || steady.contains_key(&n)) else {
                                        return Err(ctx("phantom", name, format!("handed {:?} ({} bytes), a key no thread ever wrote", String::from_utf8_lossy(k), v.len())));
                                    };
                                    toks.push((n, tok(n, v, name)?));
                                }
                                walked(sh, steady, (lo, hi), stop != 0 && seen.len() >= stop, *rev, &toks, (name, b, e), &mut recs).map_err(|(kind, why)| ctx(kind, name, why))?;
                            }
                        }
                    }
                    Ok(recs)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| {
                h.join().unwrap_or_else(|p| {
                    let msg = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()));
                    Err(fail("shlin__panic__store_thread", format!("thread panicked: {msg:?}")))
                })
            })
            .collect()
    });
    let mut all = Vec::new();
    for r in results {
        all.push(r?);
    }
    let mut fin: Vec<Option<u64>> = vec![None; shape.universe as usize];
    let mut count = 0usize;
    let mut bad: Option<Fail> = None;
    store.scan::<[u8], ()>(.., |k, v| {
        count += 1;
        let r = match snum(k, KS) {
            None => Err(format!("key {:?} nobody wrote", String::from_utf8_lossy(k))),
            Some(n) => match (shape.hot(n), steady.get(&n)) {
                (Some(i), _) => scheck(n, v).map(|tk| fin[i as usize] = Some(tk)),
                (None, Some(&w)) => scheck(n, v).and_then(|tk| if tk == w { Ok(()) } else { Err(format!("never-written key number {n} holds token {tk:#x}")) }),
                (None, None) => Err(format!("key number {n} nobody wrote")),
            },
        };
        match r {
            Ok(()) => ControlFlow::Continue(()),
            Err(why) => {
                bad = Some(fail("shlin__final__store_contents", format!("after the join: {why}")));
                ControlFlow::Break(())
            }
        }
    });
    if let Some(f) = bad {
        return Err(f);
    }
    let held = fin.iter().filter(|v| v.is_some()).count();
    if count != held + steady.len() {
        return Err(fail("shlin__final__store_contents", format!("after the join: {count} entries, {held} hot + {} never written expected", steady.len())));
    }
    judge(&shape, &init, &fin, &all)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(began: u64, ended: u64, kind: Kind) -> Rec {
        Rec { key: 0, began, ended, kind, op: "t", batch: false }
    }

    #[test]
    fn checker() {
        // Overlapping insert and read: either order.
        let a = vec![vec![r(0, 3, Kind::Put { tok: 1, old: None })], vec![r(1, 2, Kind::Read { saw: Some(1) })]];
        assert_eq!(linearizable(&a, None, Some(1), 1000), Some(true));
        // Stale read after the write ended.
        let b = vec![vec![r(0, 1, Kind::Put { tok: 1, old: None })], vec![r(2, 3, Kind::Read { saw: None })]];
        assert_eq!(linearizable(&b, None, Some(1), 1000), Some(false));
        // Two removes both saw the value.
        let c = vec![vec![r(0, 3, Kind::Remove { old: Some(7) })], vec![r(1, 2, Kind::Remove { old: Some(7) })]];
        assert_eq!(linearizable(&c, Some(7), None, 1000), Some(false));
    }
}
