// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Panics in the code a `bbtree::BBTreeMap` (and a `BBTreeStore`) calls back:
//! closures (`update`, `update_or_insert`, `get_or_insert`, `remove_if`,
//! `read`, `read_many`, `scan`, `scan_rev`, `update_or_insert_many`, a
//! store's scans), and the keys' and values' `Ord`, `Clone` and `Drop`.
//! A bomb armed on the calling thread panics at the n-th call of one kind;
//! other threads keep working on their own keys meanwhile.
//!
//! What the docs promise, and what is checked:
//!
//! - design/09-concurrency.md, "Panics": "The latch guard unlatches on
//!   unwind; there is no poisoning; a value that a panicking `update`
//!   closure left half-changed stays as it is." So after a closure panics,
//!   the map holds what the model says (the half change included), every
//!   later call returns (no latch left held: a hang), the tree passes
//!   `check_invariants`, and no value is lost or dropped twice. Batches:
//!   D49 puts their panics outside the contract, so after one the batch's
//!   keys are read back; every other key must be untouched.
//! - `Tree::destroy` (`clear`, the map's drop) and `IntoIter`'s drop: "If a
//!   key's or value's `Drop` panics, the rest is still taken apart, in the
//!   same order, before the panic goes on ... every key and value is
//!   dropped once and every page released once", in "ascending key order,
//!   each key before its value". Checked exactly: the drop log, the live
//!   counts, the pages out.
//! - D48: a panic in `Ord`, `Clone` or `Drop` elsewhere "need not leave a
//!   valid map", but "no undefined behavior, whatever user code does". After
//!   one the map is tainted: later calls run unchecked (they may panic or
//!   answer wrong), and only a double drop (keys and values carry a live
//!   mark), an abort or a hang count.
//!
//! Signatures: `syncpanic__<kind>__<call>`.

use std::cell::{Cell, RefCell};
use std::cmp::Ordering as CmpOrdering;
use std::collections::BTreeMap;
use std::fmt;
use std::ops::{Bound, ControlFlow};
use std::panic::{AssertUnwindSafe, catch_unwind, panic_any};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

use bbtree::{BBTreeMap, InlineKey, InlineKeyStrategy, InlineWord};
use bbtree_store::BBTreeStore;
use ephyra::{BoxGen, GenExt, classify, gens};
use serde::{Deserialize, Serialize};

use crate::common::{Fail, ensure, fail};
use crate::syncalloc::Budget;

// ---------- bombs ----------

/// What a bomb waits for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Closure,
    Ord,
    KeyDrop,
    ValDrop,
    KeyClone,
    ValClone,
}

/// The payload of a bomb's panic, so that the map's own panics show apart.
struct Bomb;

thread_local! {
    static ARMED: Cell<Option<(Kind, u32)>> = const { Cell::new(None) };
    static FIRED: Cell<bool> = const { Cell::new(false) };
    /// Drops seen while recording: (a key's?, its number).
    static LOG: RefCell<Option<Vec<(bool, u64)>>> = const { RefCell::new(None) };
}

fn arm(kind: Kind, after: u32) {
    ARMED.with(|a| a.set(Some((kind, after.max(1)))));
    FIRED.with(|f| f.set(false));
}

/// Disarms; whether the bomb went off.
fn disarm() -> bool {
    ARMED.with(|a| a.set(None));
    FIRED.with(|f| f.replace(false))
}

/// One call of user code of `kind`: panics if it is the one the bomb waits
/// for. Never while unwinding (a second panic aborts, by Rust's rules).
fn tick(kind: Kind) {
    if std::thread::panicking() {
        return;
    }
    let fire = ARMED.with(|a| match a.get() {
        Some((k, left)) if k == kind => {
            if left <= 1 {
                a.set(None);
                true
            } else {
                a.set(Some((k, left - 1)));
                false
            }
        }
        _ => false,
    });
    if fire {
        FIRED.with(|f| f.set(true));
        panic_any(Bomb);
    }
}

fn log_drop(key: bool, n: u64) {
    LOG.with(|l| {
        if let Some(v) = l.borrow_mut().as_mut() {
            v.push((key, n));
        }
    });
}

// ---------- tracked keys and values ----------

const ALIVE: u64 = 0x5AFE_C0DE_A11C_E000;
const DEAD: u64 = 0xDEAD_DEAD_DEAD_DEAD;

static LIVE_K: AtomicIsize = AtomicIsize::new(0);
static LIVE_V: AtomicIsize = AtomicIsize::new(0);
/// The first key or value used after its drop (or dropped twice).
static BAD: Mutex<Option<String>> = Mutex::new(None);

fn bad(msg: impl FnOnce() -> String) {
    let mut b = BAD.lock().unwrap_or_else(|e| e.into_inner());
    if b.is_none() {
        *b = Some(msg());
    }
}

fn reset() {
    LIVE_K.store(0, Ordering::Relaxed);
    LIVE_V.store(0, Ordering::Relaxed);
    *BAD.lock().unwrap_or_else(|e| e.into_inner()) = None;
    disarm();
    LOG.with(|l| *l.borrow_mut() = None);
}

fn bad_check(call: &str) -> Result<(), Fail> {
    match BAD.lock().unwrap_or_else(|e| e.into_inner()).take() {
        Some(msg) => Err(fail(&format!("syncpanic__double_drop__{call}"), msg)),
        None => Ok(()),
    }
}

/// A value: its number, a live mark, counted in `LIVE_V`.
pub struct PV {
    n: u64,
    mark: u64,
}

impl PV {
    fn new(n: u64) -> Self {
        LIVE_V.fetch_add(1, Ordering::Relaxed);
        PV { n, mark: ALIVE }
    }
    fn n(&self) -> u64 {
        if self.mark != ALIVE {
            bad(|| format!("a value read after its drop (mark {:#x}, n {})", self.mark, self.n));
        }
        self.n
    }
}

impl Clone for PV {
    fn clone(&self) -> Self {
        let n = self.n();
        tick(Kind::ValClone);
        PV::new(n)
    }
}

impl Drop for PV {
    fn drop(&mut self) {
        if self.mark != ALIVE {
            bad(|| format!("a value dropped twice (mark {:#x}, n {})", self.mark, self.n));
            return;
        }
        self.mark = DEAD;
        LIVE_V.fetch_sub(1, Ordering::Relaxed);
        log_drop(false, self.n);
        tick(Kind::ValDrop);
    }
}

impl fmt::Debug for PV {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.n)
    }
}

/// A user key under `WithInlineKey`: a u64 whose inline key is its
/// big-endian bytes, with a live mark; its `Ord`, `Clone` and `Drop` tick
/// bombs. Its `InlineKey` impl never panics (the docs: that aborts).
pub struct PK {
    k: u64,
    mark: u64,
}

impl PK {
    fn get(&self) -> u64 {
        if self.mark != ALIVE {
            bad(|| format!("a key used after its drop (mark {:#x}, k {})", self.mark, self.k));
        }
        self.k
    }
}

impl Clone for PK {
    fn clone(&self) -> Self {
        let k = self.get();
        tick(Kind::KeyClone);
        LIVE_K.fetch_add(1, Ordering::Relaxed);
        PK { k, mark: ALIVE }
    }
}

impl Drop for PK {
    fn drop(&mut self) {
        if self.mark != ALIVE {
            bad(|| format!("a key dropped twice (mark {:#x}, k {})", self.mark, self.k));
            return;
        }
        self.mark = DEAD;
        LIVE_K.fetch_sub(1, Ordering::Relaxed);
        log_drop(true, self.k);
        tick(Kind::KeyDrop);
    }
}

impl PartialEq for PK {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == CmpOrdering::Equal
    }
}
impl Eq for PK {}
impl PartialOrd for PK {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}
impl Ord for PK {
    fn cmp(&self, other: &Self) -> CmpOrdering {
        let (a, b) = (self.get(), other.get());
        tick(Kind::Ord);
        a.cmp(&b)
    }
}

impl fmt::Debug for PK {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PK({})", self.k)
    }
}

impl InlineKey for PK {
    fn inline_key_at(&self, offset: usize) -> InlineWord {
        let be = self.k.to_be_bytes();
        let mut w = [0u8; 8];
        for (i, b) in w.iter_mut().enumerate() {
            *b = offset.checked_add(i).and_then(|at| be.get(at)).copied().unwrap_or(0);
        }
        InlineWord::from_be_bytes(w)
    }
}

/// A key type for the map, made from a number and read back as one.
pub trait PKey: Ord + Clone + fmt::Debug + Send + Sync + 'static {
    /// Whether its drops are counted and logged (only `PK`'s).
    const TRACKED: bool;
    fn of(n: u64) -> Self;
    fn num(&self) -> u64;
}

impl PKey for PK {
    const TRACKED: bool = true;
    fn of(n: u64) -> Self {
        LIVE_K.fetch_add(1, Ordering::Relaxed);
        PK { k: n, mark: ALIVE }
    }
    fn num(&self) -> u64 {
        self.get()
    }
}

impl PKey for u64 {
    const TRACKED: bool = false;
    fn of(n: u64) -> Self {
        n
    }
    fn num(&self) -> u64 {
        *self
    }
}

impl PKey for String {
    const TRACKED: bool = false;
    /// Zero-padded, so that the strings sort as the numbers do.
    fn of(n: u64) -> Self {
        format!("{n:020}")
    }
    fn num(&self) -> u64 {
        self.parse().unwrap_or(u64::MAX)
    }
}

// ---------- cases ----------

/// Lanes: thread `t` owns raw keys `r * LANES + t`.
const LANES: u64 = 4;

/// Raw key numbers to keys. Shapes: dense; a shared 40-bit prefix; spread
/// over the high bytes (the shared prefix shrinks as keys come); dense near
/// the top, descending.
fn key_num(shape: u8, x: u64) -> u64 {
    match shape % 4 {
        0 => x,
        1 => (0xAB << 40) + x * 7,
        2 => (x << 40) | 0x55,
        _ => u64::MAX - x,
    }
}

fn raw_of(shape: u8, n: u64) -> u64 {
    match shape % 4 {
        0 => n,
        1 => (n - (0xAB << 40)) / 7,
        2 => n >> 40,
        _ => u64::MAX - n,
    }
}

fn lane_of(shape: u8, n: u64) -> u64 {
    raw_of(shape, n) % LANES
}

/// A call of the map's API. Raw keys are per lane; the thread maps them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Call {
    Insert(u16, u16),
    InsertMany([(u16, u16); 4]),
    /// Sets the value, then the closure ticks.
    Update(u16, u16),
    /// Adds to the value, then the closure ticks.
    UpdateOrInsert(u16, u16),
    UpdateOrInsertMany([(u16, u16); 4]),
    GetOrInsert(u16, u16),
    Remove(u16),
    /// Removes if the value is even.
    RemoveIf(u16),
    Read(u16),
    ReadMany([u16; 4]),
    GetCloned(u16),
    GetManyCloned([u16; 4]),
    Scan { a: u16, b: u16, rev: bool },
    RangeCloned { a: u16, b: u16, walk: Vec<bool> },
    /// The owner's: no other thread runs then.
    Clear,
    /// Moves the map into `into_iter`, takes this many, drops the rest.
    IntoIterDrop(u16),
    DropMap,
}

impl Call {
    fn name(&self) -> &'static str {
        match self {
            Call::Insert(..) => "insert",
            Call::InsertMany(..) => "insert_many",
            Call::Update(..) => "update",
            Call::UpdateOrInsert(..) => "update_or_insert",
            Call::UpdateOrInsertMany(..) => "update_or_insert_many",
            Call::GetOrInsert(..) => "get_or_insert",
            Call::Remove(..) => "remove",
            Call::RemoveIf(..) => "remove_if",
            Call::Read(..) => "read",
            Call::ReadMany(..) => "read_many",
            Call::GetCloned(..) => "get_cloned",
            Call::GetManyCloned(..) => "get_many_cloned",
            Call::Scan { .. } => "scan",
            Call::RangeCloned { .. } => "range_cloned",
            Call::Clear => "clear",
            Call::IntoIterDrop(..) => "into_iter_drop",
            Call::DropMap => "drop",
        }
    }
    fn owner(&self) -> bool {
        matches!(self, Call::Clear | Call::IntoIterDrop(..) | Call::DropMap)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Op {
    Plain(Call),
    Bomb(Call, Kind, u16),
    /// Whole compare, invariants and live counts (alone only).
    Check,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Case {
    shape: u8,
    /// Entries loaded first (lane 0 alone; each helper its own).
    bulk: u16,
    /// Helper threads (lanes 1..), each running `helper` over and over until
    /// the main thread is done.
    threads: u8,
    ops: Vec<Op>,
    helper: Vec<Op>,
}

fn raw() -> BoxGen<u16> {
    gens::frequency(vec![(5, gens::int(0u16..=200).boxed()), (2, gens::int(0u16..=3000).boxed()), (1, gens::int(0u16..=u16::MAX).boxed())]).boxed()
}

fn call(owner: bool) -> BoxGen<Call> {
    let k = raw();
    let v = gens::int(0u16..=1000);
    let k4 = (k.clone(), k.clone(), k.clone(), k.clone()).map(|(a, b, c, d)| [a, b, c, d]).boxed();
    let e4 = (k4.clone(), v.clone()).map(|(ks, v)| ks.map(|k| (k, v.wrapping_add(k)))).boxed();
    let walk = gens::vec(gens::bool()).len(0..=40);
    let mut alts = vec![
        (4, (k.clone(), v.clone()).map(|(k, v)| Call::Insert(k, v)).boxed()),
        (2, e4.clone().map(Call::InsertMany).boxed()),
        (4, (k.clone(), v.clone()).map(|(k, v)| Call::Update(k, v)).boxed()),
        (4, (k.clone(), v.clone()).map(|(k, v)| Call::UpdateOrInsert(k, v)).boxed()),
        (3, e4.map(Call::UpdateOrInsertMany).boxed()),
        (3, (k.clone(), v).map(|(k, v)| Call::GetOrInsert(k, v)).boxed()),
        (3, k.clone().map(Call::Remove).boxed()),
        (3, k.clone().map(Call::RemoveIf).boxed()),
        (2, k.clone().map(Call::Read).boxed()),
        (2, k4.clone().map(Call::ReadMany).boxed()),
        (1, k.clone().map(Call::GetCloned).boxed()),
        (1, k4.map(Call::GetManyCloned).boxed()),
        (3, (k.clone(), k.clone(), gens::bool()).map(|(a, b, rev)| Call::Scan { a, b, rev }).boxed()),
        (2, (k.clone(), k, walk).map(|(a, b, walk)| Call::RangeCloned { a, b, walk }).boxed()),
    ];
    if owner {
        alts.push((1, gens::constant(Call::Clear).boxed()));
        alts.push((1, gens::frequency(vec![(2, gens::int(0u16..=5).boxed()), (1, gens::int(0u16..=3000).boxed())]).map(Call::IntoIterDrop).boxed()));
        alts.push((1, gens::constant(Call::DropMap).boxed()));
    }
    gens::frequency(alts).boxed()
}

fn kind() -> BoxGen<Kind> {
    gens::frequency(vec![
        (8, gens::constant(Kind::Closure).boxed()),
        (2, gens::constant(Kind::Ord).boxed()),
        (1, gens::constant(Kind::KeyDrop).boxed()),
        (3, gens::constant(Kind::ValDrop).boxed()),
        (1, gens::constant(Kind::KeyClone).boxed()),
        (1, gens::constant(Kind::ValClone).boxed()),
    ])
    .boxed()
}

fn after() -> BoxGen<u16> {
    gens::frequency(vec![(6, gens::int(1u16..=3).boxed()), (2, gens::int(1u16..=40).boxed()), (1, gens::int(1u16..=4000).boxed())]).boxed()
}

fn op(owner: bool) -> BoxGen<Op> {
    gens::frequency(vec![
        (5, call(owner).map(Op::Plain).boxed()),
        (5, (call(owner), kind(), after()).map(|(c, k, a)| Op::Bomb(c, k, a)).boxed()),
        (1, gens::constant(Op::Check).boxed()),
    ])
    .boxed()
}

pub fn case() -> BoxGen<Case> {
    (
        gens::int(0u8..=3),
        gens::frequency(vec![(2, gens::int(0u16..=20).boxed()), (3, gens::int(0u16..=600).boxed()), (2, gens::int(0u16..=6000).boxed())]),
        gens::frequency(vec![(3, gens::constant(0u8).boxed()), (2, gens::int(1u8..=3).boxed())]),
        gens::vec(op(true)).len(1..=60),
        gens::vec(op(false)).len(1..=20),
    )
        .map(|(shape, bulk, threads, ops, helper)| Case { shape, bulk, threads, ops, helper })
        .boxed()
}

// ---------- running ----------

type Map<K, S> = BBTreeMap<K, PV, S, Budget>;

/// One thread's view: its lane, its model (raw-mapped key numbers to
/// values), and whether a user-code panic tainted the map (D48).
struct Lane {
    shape: u8,
    lane: u64,
    alone: bool,
    model: BTreeMap<u64, u64>,
    tainted: bool,
    /// Whether `get_or_insert`'s closure may panic on a new entry (the
    /// `syncvacant` properties): kept apart, so that its aborts (it runs
    /// inside a split) don't hide other aborts behind one signature.
    vacant: bool,
}

impl Lane {
    fn key(&self, raw: u16) -> u64 {
        key_num(self.shape, u64::from(raw) * LANES + self.lane)
    }
    fn mine(&self, n: u64) -> bool {
        self.alone || lane_of(self.shape, n) == self.lane
    }
}

/// What a bombed call did: its result if it returned, whether the bomb
/// went off, and the map's own panic if one escaped that was no bomb's.
struct Outcome<T> {
    out: Option<T>,
    fired: bool,
    own_panic: Option<String>,
}

fn bombed<T>(bomb: Option<(Kind, u16)>, f: impl FnOnce() -> T) -> Outcome<T> {
    if let Some((kind, after)) = bomb {
        arm(kind, u32::from(after));
    }
    let r = catch_unwind(AssertUnwindSafe(|| {
        let out = f();
        // Results (replaced values) drop in the harness, not in the map.
        let fired = disarm();
        (out, fired)
    }));
    match r {
        Ok((out, fired)) => Outcome { out: Some(out), fired, own_panic: None },
        Err(p) => {
            let fired = disarm();
            let own = if p.is::<Bomb>() {
                None
            } else {
                Some(p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default())
            };
            Outcome { out: None, fired: fired || p.is::<Bomb>(), own_panic: own }
        }
    }
}

fn cl(a: u16, b: u16) -> (u16, u16) {
    (a.min(b), a.max(b))
}

/// One call on the map, checked against the lane's model.
fn step<K: PKey, S: InlineKeyStrategy<K>>(map: &Map<K, S>, lane: &mut Lane, call: &Call, bomb: Option<(Kind, u16)>, at: &dyn Fn(String) -> String) -> Result<(), Fail> {
    let name = call.name();
    let sig = |kind: &str| format!("syncpanic__{kind}__{name}");
    let bomb = match (call, bomb) {
        (Call::GetOrInsert(..), Some((Kind::Closure, _))) if !lane.vacant => None,
        _ => bomb,
    };
    let closure_bomb = matches!(bomb, Some((Kind::Closure, _)));
    // A panic of the map's own, with no bomb of user code before it, is a
    // crash; after a user-code panic (D48) it is allowed.
    macro_rules! own {
        ($o:expr) => {
            if let Some(msg) = &$o.own_panic {
                if lane.tainted {
                    return Ok(());
                }
                return Err(fail(&sig("panic"), at(format!("the map panicked: {msg}"))));
            }
            if $o.fired && !closure_bomb {
                classify(true, "user_code_panicked");
                lane.tainted = true;
            }
            if lane.tainted {
                return Ok(());
            }
            if $o.fired {
                classify(true, "closure_panicked");
            }
        };
    }
    match call {
        Call::Insert(k, v) => {
            let n = lane.key(*k);
            let o = bombed(bomb, || map.insert(K::of(n), PV::new(u64::from(*v))).map(|old| old.map(|p| p.n())).map_err(|_| ()));
            own!(o);
            if let Some(r) = o.out {
                let want = lane.model.insert(n, u64::from(*v));
                ensure(r == Ok(want), &sig("mismatch"), || at(format!("insert returned {r:?}, model {want:?}")))?;
            }
        }
        Call::InsertMany(es) => {
            let o = bombed(bomb, || {
                let batch = es.map(|(k, v)| (K::of(lane.key(k)), PV::new(u64::from(v))));
                map.insert_many(batch).map(|r| r.map(|old| old.map(|p| p.n())).map_err(|_| ()))
            });
            own!(o);
            if let Some(rs) = o.out {
                for (i, (r, (k, v))) in rs.into_iter().zip(es).enumerate() {
                    let want = lane.model.insert(lane.key(*k), u64::from(*v));
                    ensure(r == Ok(want), &sig("mismatch"), || at(format!("entry {i}: returned {r:?}, model {want:?}")))?;
                }
            }
        }
        Call::Update(k, v) => {
            let n = lane.key(*k);
            let v = u64::from(*v);
            let o = bombed(bomb, || {
                map.update(&K::of(n), |_, stored| {
                    stored.n = v;
                    tick(Kind::Closure);
                })
            });
            own!(o);
            let held = lane.model.get_mut(&n);
            let present = held.is_some();
            if let Some(stored) = held {
                // Half-changed or done: the value is set either way.
                *stored = v;
            }
            if let Some(r) = o.out {
                ensure(r.is_some() == present, &sig("mismatch"), || at(format!("update found {}, model {present}", r.is_some())))?;
            }
            settle(map, lane, n, &sig("mismatch"), at)?;
        }
        Call::UpdateOrInsert(k, v) => {
            let n = lane.key(*k);
            let v = u64::from(*v);
            let o = bombed(bomb, || {
                map.update_or_insert(K::of(n), PV::new(v), |_, stored, add| {
                    stored.n = stored.n.wrapping_add(add.n());
                    tick(Kind::Closure);
                    stored.n
                })
                .map_err(|_| ())
            });
            own!(o);
            let want = match lane.model.get_mut(&n) {
                Some(stored) => {
                    *stored = stored.wrapping_add(v);
                    Some(*stored)
                }
                None => {
                    lane.model.insert(n, v);
                    None
                }
            };
            if let Some(r) = o.out {
                ensure(r == Ok(want), &sig("mismatch"), || at(format!("update_or_insert returned {r:?}, model {want:?}")))?;
            }
            settle(map, lane, n, &sig("mismatch"), at)?;
        }
        Call::UpdateOrInsertMany(es) => {
            let o = bombed(bomb, || {
                let batch = es.map(|(k, v)| (K::of(lane.key(k)), PV::new(u64::from(v))));
                map.update_or_insert_many(batch, |_, stored, add| {
                    stored.n = stored.n.wrapping_add(add.n());
                    tick(Kind::Closure);
                    stored.n
                })
                .map(|r| r.map_err(|_| ()))
            });
            own!(o);
            match o.out {
                Some(rs) => {
                    for (i, (r, (k, v))) in rs.into_iter().zip(es).enumerate() {
                        let v = u64::from(*v);
                        let want = match lane.model.get_mut(&lane.key(*k)) {
                            Some(stored) => {
                                *stored = stored.wrapping_add(v);
                                Some(*stored)
                            }
                            None => {
                                lane.model.insert(lane.key(*k), v);
                                None
                            }
                        };
                        ensure(r == Ok(want), &sig("mismatch"), || at(format!("entry {i}: returned {r:?}, model {want:?}")))?;
                    }
                }
                None => {
                    // Outside the contract (D49): read the batch's keys back.
                    for (k, _) in es {
                        let n = lane.key(*k);
                        match map.get_cloned(&K::of(n)) {
                            Some(p) => lane.model.insert(n, p.n()),
                            None => lane.model.remove(&n),
                        };
                    }
                }
            }
        }
        Call::GetOrInsert(k, v) => {
            let n = lane.key(*k);
            let v = u64::from(*v);
            let o = bombed(bomb, || {
                map.get_or_insert(K::of(n), PV::new(v), |_, stored| {
                    tick(Kind::Closure);
                    stored.n()
                })
                .map_err(|_| ())
            });
            own!(o);
            match o.out {
                Some(r) => {
                    let want = *lane.model.entry(n).or_insert(v);
                    ensure(r == Ok(want), &sig("mismatch"), || at(format!("get_or_insert saw {r:?}, model {want}")))?;
                }
                None if lane.model.contains_key(&n) => settle(map, lane, n, &sig("mismatch"), at)?,
                None => {
                    // f ran "just before it goes in": in or not, but once.
                    let got = map.get_cloned(&K::of(n)).map(|p| p.n());
                    ensure(got.is_none() || got == Some(v), &sig("mismatch"), || at(format!("after the panic the new key holds {got:?}, not {v} or nothing")))?;
                    classify(got.is_some(), "get_or_insert_kept_new");
                    if let Some(g) = got {
                        lane.model.insert(n, g);
                    }
                }
            }
        }
        Call::Remove(k) => {
            let n = lane.key(*k);
            let o = bombed(bomb, || map.remove(&K::of(n)).map(|p| p.n()));
            own!(o);
            if let Some(r) = o.out {
                let want = lane.model.remove(&n);
                ensure(r == want, &sig("mismatch"), || at(format!("remove {r:?}, model {want:?}")))?;
            }
        }
        Call::RemoveIf(k) => {
            let n = lane.key(*k);
            let o = bombed(bomb, || {
                map.remove_if(&K::of(n), |_, v| {
                    tick(Kind::Closure);
                    v.n() % 2 == 0
                })
                .map(|(_, p)| p.n())
            });
            own!(o);
            match o.out {
                Some(r) => {
                    let want = match lane.model.get(&n) {
                        Some(v) if v % 2 == 0 => lane.model.remove(&n),
                        _ => None,
                    };
                    ensure(r == want, &sig("mismatch"), || at(format!("remove_if {r:?}, model {want:?}")))?;
                }
                None => settle(map, lane, n, &sig("mismatch"), at)?,
            }
        }
        Call::Read(k) => {
            let n = lane.key(*k);
            let o = bombed(bomb, || {
                map.read(&K::of(n), |_, v| {
                    tick(Kind::Closure);
                    v.n()
                })
            });
            own!(o);
            if let Some(r) = o.out {
                let want = lane.model.get(&n).copied();
                ensure(r == want, &sig("mismatch"), || at(format!("read {r:?}, model {want:?}")))?;
            }
            settle(map, lane, n, &sig("mismatch"), at)?;
        }
        Call::ReadMany(ks) => {
            let ns = ks.map(|k| lane.key(k));
            let o = bombed(bomb, || {
                let keys = ns.map(K::of);
                map.read_many(keys.each_ref(), |_, v| {
                    tick(Kind::Closure);
                    v.n()
                })
            });
            own!(o);
            if let Some(r) = o.out {
                let want = ns.map(|n| lane.model.get(&n).copied());
                ensure(r == want, &sig("mismatch"), || at(format!("read_many {r:?}, model {want:?}")))?;
            }
            for n in ns {
                settle(map, lane, n, &sig("mismatch"), at)?;
            }
        }
        Call::GetCloned(k) => {
            let n = lane.key(*k);
            let o = bombed(bomb, || map.get_cloned(&K::of(n)).map(|p| p.n()));
            own!(o);
            if let Some(r) = o.out {
                let want = lane.model.get(&n).copied();
                ensure(r == want, &sig("mismatch"), || at(format!("get_cloned {r:?}, model {want:?}")))?;
            }
        }
        Call::GetManyCloned(ks) => {
            let ns = ks.map(|k| lane.key(k));
            let o = bombed(bomb, || {
                let keys = ns.map(K::of);
                map.get_many_cloned(keys.each_ref()).map(|r| r.map(|p| p.n()))
            });
            own!(o);
            if let Some(r) = o.out {
                let want = ns.map(|n| lane.model.get(&n).copied());
                ensure(r == want, &sig("mismatch"), || at(format!("get_many_cloned {r:?}, model {want:?}")))?;
            }
        }
        Call::Scan { a, b, rev } => {
            let (a, b) = cl(*a, *b);
            let (lo, hi) = (lane.key(a), lane.key(b));
            let (lo, hi) = (lo.min(hi), lo.max(hi));
            let o = bombed(bomb, || {
                let mut seen = Vec::new();
                let (s, e) = (Bound::Included(K::of(lo)), Bound::Included(K::of(hi)));
                let f = |k: &K, v: &PV| {
                    seen.push((k.num(), v.n()));
                    tick(Kind::Closure);
                    ControlFlow::<()>::Continue(())
                };
                if *rev {
                    map.scan_rev((s, e), f);
                } else {
                    map.scan((s, e), f);
                }
                seen
            });
            own!(o);
            if let Some(mut seen) = o.out {
                seen.retain(|(n, _)| lane.mine(*n));
                let mut want: Vec<_> = lane.model.range(lo..=hi).map(|(k, v)| (*k, *v)).collect();
                if *rev {
                    want.reverse();
                }
                ensure(seen == want, &sig("mismatch"), || at(format!("scan [{lo}, {hi}] rev {rev}: {} entries, model {}; {:?} vs {:?}", seen.len(), want.len(), first_diff(&seen, &want), want.len())))?;
            }
        }
        Call::RangeCloned { a, b, walk } => {
            let (a, b) = cl(*a, *b);
            let (lo, hi) = (lane.key(a), lane.key(b));
            let (lo, hi) = (lo.min(hi), lo.max(hi));
            let o = bombed(bomb, || {
                let mut it = map.range_cloned::<K, _>((Bound::Included(K::of(lo)), Bound::Excluded(K::of(hi))));
                let (mut front, mut back) = (Vec::new(), Vec::new());
                for &f in walk {
                    let e = if f { it.next() } else { it.next_back() };
                    match e {
                        Some((k, v)) if f => front.push((k.num(), v.n())),
                        Some((k, v)) => back.push((k.num(), v.n())),
                        None => break,
                    }
                }
                front.extend(it.map(|(k, v)| (k.num(), v.n())));
                back.reverse();
                front.extend(back);
                front
            });
            own!(o);
            if let Some(mut seen) = o.out {
                seen.retain(|(n, _)| lane.mine(*n));
                let want: Vec<_> = lane.model.range(lo..hi).map(|(k, v)| (*k, *v)).collect();
                ensure(seen == want, &sig("mismatch"), || at(format!("range_cloned [{lo}, {hi}): {} entries, model {}; {:?}", seen.len(), want.len(), first_diff(&seen, &want))))?;
            }
        }
        Call::Clear | Call::IntoIterDrop(_) | Call::DropMap => unreachable!("owner calls run in run_alone"),
    }
    Ok(())
}

fn first_diff(a: &[(u64, u64)], b: &[(u64, u64)]) -> Option<(usize, Option<(u64, u64)>, Option<(u64, u64)>)> {
    (0..a.len().max(b.len())).find(|&i| a.get(i) != b.get(i)).map(|i| (i, a.get(i).copied(), b.get(i).copied()))
}

/// After a call that panicked or ran a closure: the key holds what the
/// model says.
fn settle<K: PKey, S: InlineKeyStrategy<K>>(map: &Map<K, S>, lane: &Lane, n: u64, sig: &str, at: &dyn Fn(String) -> String) -> Result<(), Fail> {
    let got = map.get_cloned(&K::of(n)).map(|p| p.n());
    let want = lane.model.get(&n).copied();
    ensure(got == want, sig, || at(format!("afterwards {n} holds {got:?}, model {want:?}")))
}

/// Whole compare, invariants, live counts: alone, untainted.
fn check_whole<K: PKey, S: InlineKeyStrategy<K>>(map: &mut Map<K, S>, model: &BTreeMap<u64, u64>, at: &dyn Fn(String) -> String) -> Result<(), Fail> {
    let all: Vec<_> = map.range_cloned::<K, _>(..).map(|(k, v)| (k.num(), v.n())).collect();
    let want: Vec<_> = model.iter().map(|(k, v)| (*k, *v)).collect();
    ensure(all == want, "syncpanic__mismatch__whole", || at(format!("{} entries, model {}; first difference {:?}", all.len(), want.len(), first_diff(&all, &want))))?;
    let len = model.len();
    if let Err(p) = catch_unwind(AssertUnwindSafe(|| map.check_invariants(len))) {
        let msg = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
        return Err(fail("syncpanic__invariant__check", at(msg)));
    }
    let (lv, lk) = (LIVE_V.load(Ordering::Relaxed), LIVE_K.load(Ordering::Relaxed));
    let want_k = if K::TRACKED { len as isize } else { 0 };
    ensure(lv == len as isize && lk == want_k, "syncpanic__leak__values", || at(format!("{lv} values and {lk} keys alive, the map holds {len}")))
}

/// What a teardown of `model` must drop, in order (D1): each key before
/// its value, ascending.
fn teardown_log<K: PKey>(model: &BTreeMap<u64, u64>) -> Vec<(bool, u64)> {
    let mut v = Vec::new();
    for (k, val) in model {
        if K::TRACKED {
            v.push((true, *k));
        }
        v.push((false, *val));
    }
    v
}

fn run_alone<K: PKey, S: InlineKeyStrategy<K>>(case: &Case, budget: &Budget, map: &mut Option<Map<K, S>>, lane: &mut Lane) -> Result<(), Fail> {
    for (i, op) in case.ops.iter().enumerate() {
        let at = |why: String| format!("op {i} {op:?}: {why}");
        let (c, bomb) = match op {
            Op::Plain(c) => (c, None),
            Op::Bomb(c, k, a) => (c, Some((*k, *a))),
            Op::Check => {
                if !lane.tainted {
                    check_whole(map.as_mut().expect("map"), &lane.model, &at)?;
                }
                continue;
            }
        };
        if c.owner() {
            let name = c.name();
            let sig = |kind: &str| format!("syncpanic__{kind}__{name}");
            let mut m = map.take().expect("map");
            let mut taken = Vec::new();
            if let Call::IntoIterDrop(n) = c {
                let mut it = m.into_iter();
                let o = bombed(None, || it.by_ref().take(usize::from(*n)).map(|(k, v)| (k.num(), v.n())).collect::<Vec<_>>());
                taken = o.out.unwrap_or_default();
                LOG.with(|l| *l.borrow_mut() = Some(Vec::new()));
                let o = bombed(bomb, || drop(it));
                let log = LOG.with(|l| l.borrow_mut().take()).unwrap_or_default();
                *map = Some(Map::<K, S>::new_in(budget.clone()));
                finish_teardown::<K>(lane, &sig, o.fired, o.own_panic, log, &taken, budget, &at)?;
            } else {
                LOG.with(|l| *l.borrow_mut() = Some(Vec::new()));
                let o = bombed(bomb, || {
                    if matches!(c, Call::Clear) {
                        m.clear();
                        Some(m)
                    } else {
                        drop(m);
                        None
                    }
                });
                let log = LOG.with(|l| l.borrow_mut().take()).unwrap_or_default();
                // A map cleared by a panicking clear is gone with the
                // unwind: the closure owned it.
                *map = Some(o.out.flatten().unwrap_or_else(|| Map::<K, S>::new_in(budget.clone())));
                finish_teardown::<K>(lane, &sig, o.fired, o.own_panic, log, &taken, budget, &at)?;
            }
            continue;
        }
        let m = map.as_ref().expect("map");
        if lane.tainted {
            let _ = catch_unwind(AssertUnwindSafe(|| step(m, lane, c, None, &at)));
        } else {
            step(m, lane, c, bomb, &at)?;
        }
        bad_check(c.name())?;
    }
    Ok(())
}

/// After a teardown (clear, drop, the rest of an `into_iter`): with an
/// untainted map, every entry dropped once in key order, nothing alive,
/// no page out (the docs of `Tree::destroy` and `DyingWalk`).
#[allow(clippy::too_many_arguments)]
fn finish_teardown<K: PKey>(lane: &mut Lane, sig: &dyn Fn(&str) -> String, fired: bool, own_panic: Option<String>, log: Vec<(bool, u64)>, taken: &[(u64, u64)], budget: &Budget, at: &dyn Fn(String) -> String) -> Result<(), Fail> {
    bad_check("teardown")?;
    let model = std::mem::take(&mut lane.model);
    if lane.tainted {
        lane.tainted = false;
        reset_counts();
        return Ok(());
    }
    classify(fired, "teardown_panicked");
    if let Some(msg) = own_panic {
        return Err(fail(&sig("panic"), at(format!("the map panicked: {msg}"))));
    }
    let want_taken: Vec<_> = model.iter().take(taken.len()).map(|(k, v)| (*k, *v)).collect();
    ensure(taken == want_taken, &sig("mismatch"), || at(format!("into_iter handed out {:?}, model {:?}", first_diff(taken, &want_taken), want_taken.len())))?;
    let rest: BTreeMap<u64, u64> = model.iter().skip(taken.len()).map(|(k, v)| (*k, *v)).collect();
    let want = teardown_log::<K>(&rest);
    if log != want {
        let i = (0..log.len().max(want.len())).find(|&i| log.get(i) != want.get(i));
        return Err(fail(&sig("drop_order"), at(format!("{} drops, want {}; first difference at {i:?}: {:?} vs {:?}", log.len(), want.len(), i.and_then(|i| log.get(i)), i.and_then(|i| want.get(i))))));
    }
    let (lv, lk, out) = (LIVE_V.load(Ordering::Relaxed), LIVE_K.load(Ordering::Relaxed), budget.out());
    ensure(lv == 0 && lk == 0 && out == 0, &sig("leak"), || at(format!("after the teardown {lv} values, {lk} keys alive, {out} pages out")))?;
    Ok(())
}

fn reset_counts() {
    LIVE_K.store(0, Ordering::Relaxed);
    LIVE_V.store(0, Ordering::Relaxed);
}

/// A helper thread: its own lane, its ops over and over until `stop`, with
/// closure bombs only (a user-code panic may leave the map invalid, D48,
/// and the others would see it).
fn helper<K: PKey, S: InlineKeyStrategy<K>>(map: &Map<K, S>, mut lane: Lane, ops: &[Op], stop: &AtomicBool) -> Result<(), Fail> {
    let (mut round, id) = (0, lane.lane);
    while round < 2 || (!stop.load(Ordering::Relaxed) && round < 200) {
        for (i, op) in ops.iter().enumerate() {
            let at = |why: String| format!("lane {id} round {round} op {i} {op:?}: {why}");
            match op {
                Op::Plain(c) => step(map, &mut lane, c, None, &at)?,
                Op::Bomb(c, _, a) => step(map, &mut lane, c, Some((Kind::Closure, *a)), &at)?,
                Op::Check => {}
            }
        }
        round += 1;
    }
    // Its keys hold its model.
    let all: Vec<_> = map.range_cloned::<K, _>(..).map(|(k, v)| (k.num(), v.n())).filter(|(n, _)| lane.mine(*n)).collect();
    let want: Vec<_> = lane.model.iter().map(|(k, v)| (*k, *v)).collect();
    ensure(all == want, "syncpanic__mismatch__neighbor", || format!("lane {}: {} entries, model {}; {:?}", lane.lane, all.len(), want.len(), first_diff(&all, &want)))
}

pub fn run<K: PKey, S: InlineKeyStrategy<K> + Send + Sync, const VACANT: bool>(case: &Case) -> Result<(), Fail>
where
    S: InlineKeyStrategy<K>,
{
    reset();
    let budget = Budget::new();
    let threads = case.threads.min(3);
    let mut lane = Lane { shape: case.shape, lane: 0, alone: threads == 0, model: BTreeMap::new(), tainted: false, vacant: VACANT };
    let mut map = Some(Map::<K, S>::new_in(budget.clone()));
    {
        let m = map.as_ref().expect("map");
        for i in 0..case.bulk {
            let raw = (u32::from(i) * 13 % 6000) as u16;
            let n = lane.key(raw);
            m.insert(K::of(n), PV::new(u64::from(i))).map_err(|_| fail("syncpanic__error__bulk", "a bulk insert was refused"))?;
            lane.model.insert(n, u64::from(i));
        }
    }
    if threads == 0 {
        let r = run_alone::<K, S>(case, &budget, &mut map, &mut lane);
        bad_check("end")?;
        r?;
    } else {
        classify(true, "threads");
        let stop = AtomicBool::new(false);
        let m = map.as_ref().expect("map");
        let r = std::thread::scope(|s| {
            let hs: Vec<_> = (1..=u64::from(threads))
                .map(|t| {
                    let (stop, ops) = (&stop, &case.helper);
                    s.spawn(move || {
                        let mut l = Lane { shape: case.shape, lane: t, alone: false, model: BTreeMap::new(), tainted: false, vacant: VACANT };
                        for i in 0..case.bulk / 2 {
                            let raw = (u32::from(i) * 17 % 6000) as u16;
                            let n = l.key(raw);
                            m.insert(K::of(n), PV::new(u64::from(i))).map_err(|_| fail("syncpanic__error__bulk", "a bulk insert was refused"))?;
                            l.model.insert(n, u64::from(i));
                        }
                        helper(m, l, ops, stop)
                    })
                })
                .collect();
            let mut r = Ok(());
            for (i, op) in case.ops.iter().enumerate() {
                let at = |why: String| format!("lane 0 op {i} {op:?}: {why}");
                let res = match op {
                    // The owner's calls wait for the threads: skipped here.
                    Op::Plain(c) | Op::Bomb(c, ..) if c.owner() => Ok(()),
                    Op::Plain(c) => step(m, &mut lane, c, None, &at),
                    Op::Bomb(c, _, a) => step(m, &mut lane, c, Some((Kind::Closure, *a)), &at),
                    Op::Check => Ok(()),
                };
                if res.is_err() {
                    r = res;
                    break;
                }
            }
            stop.store(true, Ordering::Relaxed);
            for h in hs {
                let hr = h.join().unwrap_or_else(|_| Err(fail("syncpanic__panic__thread", "a helper thread panicked")));
                if r.is_ok() {
                    r = hr;
                }
            }
            r
        });
        bad_check("threads")?;
        r?;
        // Lane 0's keys hold its model; then the whole tree.
        let mut mm = map.take().expect("map");
        let all: Vec<_> = mm.range_cloned::<K, _>(..).map(|(k, v)| (k.num(), v.n())).filter(|(n, _)| lane.mine(*n)).collect();
        let want: Vec<_> = lane.model.iter().map(|(k, v)| (*k, *v)).collect();
        ensure(all == want, "syncpanic__mismatch__whole", || format!("lane 0: {} entries, model {}; {:?}", all.len(), want.len(), first_diff(&all, &want)))?;
        let len = mm.range_cloned::<K, _>(..).count();
        if let Err(p) = catch_unwind(AssertUnwindSafe(|| mm.check_invariants(len))) {
            let msg = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
            return Err(fail("syncpanic__invariant__check", format!("after the threads: {msg}")));
        }
        let lv = LIVE_V.load(Ordering::Relaxed);
        ensure(lv == len as isize, "syncpanic__leak__values", || format!("after the threads {lv} values alive, the map holds {len}"))?;
        map = Some(mm);
    }
    let tainted = lane.tainted;
    let dropped = catch_unwind(AssertUnwindSafe(|| drop(map.take())));
    bad_check("drop")?;
    if !tainted {
        ensure(dropped.is_ok(), "syncpanic__panic__drop", || "dropping the map panicked".into())?;
        let (lv, lk, out) = (LIVE_V.load(Ordering::Relaxed), LIVE_K.load(Ordering::Relaxed), budget.out());
        ensure(lv == 0 && lk == 0 && out == 0, "syncpanic__leak__drop", || format!("after the drop {lv} values, {lk} keys alive, {out} pages out"))?;
    }
    Ok(())
}

// ---------- the store's scans ----------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SOp {
    Insert(u16, u16),
    Remove(u16),
    /// A scan whose closure panics at the `after`-th entry (0: never).
    Scan { a: u16, b: u16, rev: bool, after: u16 },
    /// A double-ended copy of a range, compared whole.
    Range { a: u16, b: u16 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SCase {
    bulk: u16,
    threads: u8,
    ops: Vec<SOp>,
}

pub fn scase() -> BoxGen<SCase> {
    let k = raw();
    let sop = gens::frequency(vec![
        (4, (k.clone(), gens::int(0u16..=3000)).map(|(k, v)| SOp::Insert(k, v)).boxed()),
        (2, k.clone().map(SOp::Remove).boxed()),
        (5, (k.clone(), k.clone(), gens::bool(), after()).map(|(a, b, rev, after)| SOp::Scan { a, b, rev, after }).boxed()),
        (1, (k.clone(), k).map(|(a, b)| SOp::Range { a, b }).boxed()),
    ]);
    (
        gens::frequency(vec![(2, gens::int(0u16..=50).boxed()), (2, gens::int(0u16..=3000).boxed())]),
        gens::frequency(vec![(2, gens::constant(0u8).boxed()), (2, gens::int(1u8..=3).boxed())]),
        gens::vec(sop).len(1..=60),
    )
        .map(|(bulk, threads, ops)| SCase { bulk, threads, ops })
        .boxed()
}

fn skey(lane: u64, raw: u16) -> Vec<u8> {
    let x = u64::from(raw) * LANES + lane;
    let mut k = b"k/".to_vec();
    k.extend(x.to_be_bytes());
    k
}

/// A value whose length and bytes come from its number: long ones go to
/// overflow pages.
fn sval(v: u16) -> Vec<u8> {
    let len = match v % 4 {
        0 => usize::from(v) * 7,
        _ => usize::from(v % 64),
    };
    (0..len).map(|i| (v as usize + i) as u8).collect()
}

fn store_lane(store: &BBTreeStore<Budget>, lane: u64, ops: &[SOp], rounds: usize, stop: Option<&AtomicBool>) -> Result<(), Fail> {
    let mut model: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
    let mine = |k: &[u8]| k.len() == 10 && u64::from_be_bytes(k[2..10].try_into().unwrap_or_default()) % LANES == lane;
    let mut round = 0;
    while round < rounds.max(1) || stop.is_some_and(|s| !s.load(Ordering::Relaxed) && round < 200) {
        for (i, op) in ops.iter().enumerate() {
            let at = |why: String| format!("lane {lane} round {round} op {i} {op:?}: {why}");
            match op {
                SOp::Insert(k, v) => {
                    let (k, v) = (skey(lane, *k), sval(*v));
                    let r = store.insert(&k, &v).map_err(|e| format!("{e:?}"));
                    let want = model.insert(k, v).is_some();
                    ensure(r == Ok(want), "syncpanic__mismatch__store_insert", || at(format!("insert {r:?}, model replaced {want}")))?;
                }
                SOp::Remove(k) => {
                    let k = skey(lane, *k);
                    let r = store.remove(&k);
                    let want = model.remove(&k).is_some();
                    ensure(r == want, "syncpanic__mismatch__store_remove", || at(format!("remove {r}, model {want}")))?;
                }
                SOp::Scan { a, b, rev, after } => {
                    let (a, b) = cl(*a, *b);
                    let (lo, hi) = (skey(lane, a), skey(lane, b));
                    let mut seen = Vec::new();
                    if *after > 0 {
                        arm(Kind::Closure, u32::from(*after));
                    }
                    let r = catch_unwind(AssertUnwindSafe(|| {
                        let f = |k: &[u8], v: &[u8]| {
                            if mine(k) {
                                seen.push((k.to_vec(), v.to_vec()));
                            }
                            tick(Kind::Closure);
                            ControlFlow::<()>::Continue(())
                        };
                        if *rev {
                            store.scan_rev(lo.as_slice()..=hi.as_slice(), f);
                        } else {
                            store.scan(lo.as_slice()..=hi.as_slice(), f);
                        }
                    }));
                    let fired = disarm();
                    match r {
                        Ok(()) => {
                            let mut want: Vec<_> = model.range(lo.clone()..=hi.clone()).map(|(k, v)| (k.clone(), v.clone())).collect();
                            if *rev {
                                want.reverse();
                            }
                            ensure(seen == want, "syncpanic__mismatch__store_scan", || at(format!("scan {} entries, model {}", seen.len(), want.len())))?;
                        }
                        Err(p) if p.is::<Bomb>() || fired => classify(true, "store_scan_panicked"),
                        Err(_) => return Err(fail("syncpanic__panic__store_scan", at("the store panicked".into()))),
                    }
                }
                SOp::Range { a, b } => {
                    let (a, b) = cl(*a, *b);
                    let (lo, hi) = (skey(lane, a), skey(lane, b));
                    let got: Vec<_> = store.range(lo.as_slice()..hi.as_slice()).filter(|(k, _)| mine(k)).collect();
                    let want: Vec<_> = model.range(lo..hi).map(|(k, v)| (k.clone(), v.clone())).collect();
                    ensure(got == want, "syncpanic__mismatch__store_range", || at(format!("range {} entries, model {}", got.len(), want.len())))?;
                }
            }
        }
        round += 1;
    }
    for (k, v) in &model {
        let got = store.get(k);
        ensure(got.as_ref() == Some(v), "syncpanic__mismatch__store_final", || format!("lane {lane}: {k:?} holds {:?} bytes, model {}", got.map(|g| g.len()), v.len()))?;
    }
    let all: Vec<_> = store.range::<[u8], _>(..).filter(|(k, _)| mine(k)).map(|(k, _)| k).collect();
    ensure(all.len() == model.len(), "syncpanic__mismatch__store_final", || format!("lane {lane}: {} keys, model {}", all.len(), model.len()))
}

pub fn run_store(case: &SCase) -> Result<(), Fail> {
    reset();
    let budget = Budget::new();
    {
        let store = BBTreeStore::new_in(budget.clone());
        for i in 0..case.bulk {
            let k = skey(LANES - 1, i.wrapping_mul(13) % 6000);
            store.insert(&k, &sval(i)).map_err(|e| fail("syncpanic__error__store_bulk", format!("{e:?}")))?;
        }
        if case.threads == 0 {
            store_lane(&store, 0, &case.ops, 1, None)?;
        } else {
            classify(true, "store_threads");
            let stop = AtomicBool::new(false);
            std::thread::scope(|s| {
                let hs: Vec<_> = (1..=u64::from(case.threads.min(2))).map(|t| {
                    let (store, stop, ops) = (&store, &stop, &case.ops);
                    s.spawn(move || store_lane(store, t, ops, 2, Some(stop)))
                }).collect();
                let r = store_lane(&store, 0, &case.ops, 1, None);
                stop.store(true, Ordering::Relaxed);
                let mut r = r;
                for h in hs {
                    let hr = h.join().unwrap_or_else(|_| Err(fail("syncpanic__panic__store_thread", "a store thread panicked")));
                    if r.is_ok() {
                        r = hr;
                    }
                }
                r
            })?;
        }
    }
    let out = budget.out();
    ensure(out == 0, "syncpanic__leak__store_drop", || format!("after the drop {out} pages out"))
}
