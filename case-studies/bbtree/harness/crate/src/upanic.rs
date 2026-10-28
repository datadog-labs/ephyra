// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Panics in the user code an `unsync::BBTreeMap` / `BBTreeSet` calls (a
//! key's `Ord` and `InlineKey`, a key's or value's `Drop`, `extract_if` /
//! `retain` closures, the iterator `extend` pulls from), checked against
//! the exact states the unsync docs promise after one. sametree's bomb keys
//! check only soundness after a panic (D48); these docs promise more:
//!
//! - `insert` / `remove` / `remove_entry` (`unsync/map/mod.rs`): "If `K`'s
//!   `Ord` panics, the map is unchanged: every call to it happens before the
//!   first change." `InlineKey` calls made last (fitting split or merged
//!   nodes) leave the map with the entry in (insert) or out (remove);
//!   "Either way, it stays valid."
//! - `entry` (`entry.rs`): "If `K`'s `Ord`, or its `InlineKey` with
//!   `WithInlineKey`, panics here, the map is unchanged."
//! - `append` (`append.rs`): "If `K`'s `Ord` panics, `self` keeps its
//!   entries and those of `other` merged so far, and the rest of `other` is
//!   dropped" (merged so far: a prefix of `other` in key order).
//! - `split_off`: "Every `Ord` call happens first, before any entry moves.
//!   `InlineKey` is called last, once both maps are counted: if it panics,
//!   both stay valid."
//! - `extract_if` (`retain` is `extract_if(..)` negated): "If the closure
//!   returns `false`, or panics, the element remains in the map and will not
//!   be yielded"; values the closure changed stay changed.
//! - `extend` (`from_entries.rs`): "Inserts each entry in turn, as `insert`
//!   does. Large trees may pull sixteen entries ahead; a panic can drop
//!   entries already pulled from the iterator but not yet inserted."
//! - `clear`, the map's drop, `IntoIter`'s drop (`tree/teardown.rs`,
//!   `tree/walk.rs`): every key and value "in ascending key order, each key
//!   before its value ... If one of their `Drop`s panics, the rest are still
//!   dropped, in the same order", "the map is left empty", "every key and
//!   value is dropped once and every page released once".
//! - Set `insert` / `remove` / `replace` / `take`: as the map's.
//!
//! After every op: `check_invariants` / `check_prefix_lens`, `len` against
//! the walk, and a ledger of every key and value alive (by id): exactly the
//! map's and the set's, none dropped twice. At the end, no pages out.
//!
//! Keys: integers under `Auto` (user type: plain nodes, every comparison is
//! `Ord`), byte strings with the std impl's tags and `cmp_from` (only `Ord`
//! panics: those `InlineKey` calls inside moves abort by design), and byte
//! strings with the trait's defaults whose `inline_key_at` panics too.
//!
//! Signatures: `upanic__<state|mismatch|invariant|leak|double_drop|drop_order|len|panic>__<op>`.

use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fmt::Debug;
use std::ops::Bound;
use std::panic::{AssertUnwindSafe, catch_unwind};

use bbtree::InlineKeyStrategy;
use bbtree::unsync::{BBTreeMap, BBTreeSet};
use ephyra::{BoxGen, GenExt, classify, gens};
use serde::{Deserialize, Serialize};

use crate::common::{Fail, ensure, fail, map_len};
use crate::umodel::{Pages, pages};

thread_local! {
    /// Calls left before one panics (0: off): `Ord`, `InlineKey`, `Drop`, closures.
    static ORD: Cell<u32> = const { Cell::new(0) };
    static IK: Cell<u32> = const { Cell::new(0) };
    static DROPS: Cell<u32> = const { Cell::new(0) };
    static CLOSURE: Cell<u32> = const { Cell::new(0) };
    /// Which bomb went off in this op: 0 none, 1 Ord, 2 InlineKey, 3 Drop, 4 closure.
    static FIRED: Cell<u8> = const { Cell::new(0) };
    /// What went off in the last armed op.
    static LAST: Cell<u8> = const { Cell::new(0) };
    static NEXT_ID: Cell<u64> = const { Cell::new(0) };
    static LIVE: RefCell<HashSet<u64>> = RefCell::new(HashSet::new());
    static DOUBLE: Cell<u64> = const { Cell::new(0) };
    /// Drops in order while on: (is a value, number).
    static LOG: RefCell<Option<Vec<(bool, u64)>>> = const { RefCell::new(None) };
}

fn tick(cell: &'static std::thread::LocalKey<Cell<u32>>, code: u8, what: &str) {
    if std::thread::panicking() {
        return;
    }
    cell.with(|c| match c.get() {
        0 => {}
        1 => {
            c.set(0);
            FIRED.with(|f| f.set(code));
            panic!("upanic: {what} panics (the bomb)");
        }
        n => c.set(n - 1),
    })
}

fn born() -> u64 {
    let id = NEXT_ID.with(|n| {
        let id = n.get();
        n.set(id + 1);
        id
    });
    LIVE.with(|l| l.borrow_mut().insert(id));
    id
}

fn died(id: u64, value: bool, n: u64) {
    if !LIVE.with(|l| l.borrow_mut().remove(&id)) {
        DOUBLE.with(|d| d.set(d.get() + 1));
    }
    LOG.with(|l| {
        if let Some(log) = l.borrow_mut().as_mut() {
            log.push((value, n));
        }
    });
    tick(&DROPS, 3, "a Drop");
}

/// What a key type needs here: made from a number (order-preserving), read back, with an id.
pub trait PKey: Ord + Debug + 'static {
    fn make(n: u64, mode: u8) -> Self;
    fn n(&self) -> u64;
    fn id(&self) -> u64;
}

/// An integer key; under `Auto` a user type has no inline keys.
#[derive(Debug)]
pub struct IntK {
    n: u64,
    id: u64,
}
impl PartialEq for IntK {
    fn eq(&self, o: &Self) -> bool {
        self.cmp(o) == Ordering::Equal
    }
}
impl Eq for IntK {}
impl PartialOrd for IntK {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for IntK {
    fn cmp(&self, o: &Self) -> Ordering {
        tick(&ORD, 1, "Ord");
        self.n.cmp(&o.n)
    }
}
impl Drop for IntK {
    fn drop(&mut self) {
        died(self.id, false, self.n);
    }
}
impl PKey for IntK {
    fn make(n: u64, _mode: u8) -> Self {
        IntK { n, id: born() }
    }
    fn n(&self) -> u64 {
        self.n
    }
    fn id(&self) -> u64 {
        self.id
    }
}

/// The bytes of key `n`: a shared prefix (none, short, past the 64 bytes a
/// search checks), then `n` big-endian, so that bytes order as numbers.
fn key_bytes(n: u64, mode: u8) -> Vec<u8> {
    let mut b = match mode % 3 {
        0 => Vec::new(),
        1 => b"tenant/".to_vec(),
        _ => vec![b'p'; 70],
    };
    b.extend_from_slice(&n.to_be_bytes());
    b
}

macro_rules! str_key {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug)]
        pub struct $name {
            b: Vec<u8>,
            n: u64,
            id: u64,
        }
        impl PartialEq for $name {
            fn eq(&self, o: &Self) -> bool {
                self.cmp(o) == Ordering::Equal
            }
        }
        impl Eq for $name {}
        impl PartialOrd for $name {
            fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
                Some(self.cmp(o))
            }
        }
        impl Ord for $name {
            fn cmp(&self, o: &Self) -> Ordering {
                tick(&ORD, 1, "Ord");
                self.b.cmp(&o.b)
            }
        }
        impl Drop for $name {
            fn drop(&mut self) {
                died(self.id, false, self.n);
            }
        }
    };
}

str_key!(StrK, "A byte-string key with the std impl's hash tags and `cmp_from`; only its `Ord` panics.");
str_key!(IkK, "A byte-string key with the trait's defaults (no tags, no `cmp_from`); `Ord` and `inline_key_at` panic.");

impl bbtree::InlineKey for StrK {
    const HAS_HASH_TAG: bool = true;
    const HAS_CMP_FROM: bool = true;
    fn inline_key_at(&self, offset: usize) -> bbtree::InlineWord {
        self.b.inline_key_at(offset)
    }
    fn hash_tag(&self) -> u8 {
        self.b.hash_tag()
    }
    fn cmp_from(&self, other: &Self, equal_len: usize) -> Ordering {
        self.b.cmp_from(&other.b, equal_len)
    }
    fn cmp_prefix(&self, prefix: &[u8]) -> Ordering {
        self.b.cmp_prefix(prefix)
    }
}
impl PKey for StrK {
    fn make(n: u64, mode: u8) -> Self {
        StrK { b: key_bytes(n, mode), n, id: born() }
    }
    fn n(&self) -> u64 {
        self.n
    }
    fn id(&self) -> u64 {
        self.id
    }
}

impl bbtree::InlineKey for IkK {
    fn inline_key_at(&self, offset: usize) -> bbtree::InlineWord {
        tick(&IK, 2, "InlineKey");
        self.b.inline_key_at(offset)
    }
}
impl PKey for IkK {
    fn make(n: u64, mode: u8) -> Self {
        IkK { b: key_bytes(n, mode), n, id: born() }
    }
    fn n(&self) -> u64 {
        self.n
    }
    fn id(&self) -> u64 {
        self.id
    }
}

/// A value with an id whose `Drop` can panic.
#[derive(Debug)]
pub struct TV {
    n: u64,
    id: u64,
}
impl TV {
    fn new(n: u64) -> Self {
        TV { n, id: born() }
    }
}
impl Drop for TV {
    fn drop(&mut self) {
        died(self.id, true, self.n);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Bomb {
    None,
    Ord(u16),
    Ik(u16),
    Drop(u16),
    Closure(u16),
}

/// A key: near a stored one (the `x`-th, moved by -1..=1), or anywhere in the span.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct KP {
    near: bool,
    x: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Op {
    Insert { k: KP, v: u32, bomb: Bomb },
    Remove { k: KP, entry: bool, bomb: Bomb },
    /// how: 0 or_insert, 1 and_modify, 2 occupied remove_entry, 3 occupied insert.
    Entry { k: KP, how: u8, v: u32, bomb: Bomb },
    Pop { last: bool, bomb: Bomb },
    /// `append` of a map of `len` keys from `start` by `stride`.
    Append { start: KP, stride: u8, len: u16, bomb: Bomb },
    SplitOff { k: KP, upper: bool, bomb: Bomb },
    /// Removes keys with (n + r) % m == 0 in `lo..` / `lo..hi`, pulling at most `take`; bumps every visited value if `bump`.
    ExtractIf { lo: KP, hi: Option<KP>, m: u8, r: u8, take: u16, bump: bool, bomb: Bomb },
    Retain { m: u8, r: u8, bump: bool, bomb: Bomb },
    InsertMany { keys: Vec<KP>, v: u32, bomb: Bomb },
    /// `extend` with `len` keys from `start` by `stride` (descending if `down`);
    /// a closure bomb makes the iterator panic at that item.
    Extend { start: KP, stride: u8, len: u16, down: bool, bomb: Bomb },
    Clear { bomb: Bomb },
    Drop { bomb: Bomb },
    /// `into_iter`, `front` items from the front, `back` from the back, then the rest dropped.
    IntoIter { front: u16, back: u16, bomb: Bomb },
    FromIter { start: KP, stride: u8, len: u16, bomb: Bomb },
    /// The set's `append` (of `len` keys from `k` by 1..=3), `split_off` at `k`, `retain` of (n % m != 0).
    SetBulk { which: u8, k: KP, len: u16, m: u8, bomb: Bomb },
    /// which: 0 insert, 1 remove, 2 replace, 3 take.
    Set { which: u8, k: KP, bomb: Bomb },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Case {
    pub mode: u8,
    pub start: u32,
    pub stride: u16,
    pub len: u16,
    pub set_len: u16,
    pub ops: Vec<Op>,
}

fn bomb() -> BoxGen<Bomb> {
    let n = gens::frequency(vec![(4, gens::int(1u16..=12).boxed()), (3, gens::int(1u16..=60).boxed()), (1, gens::int(1u16..=600).boxed())]);
    gens::frequency(vec![
        (2, gens::constant(Bomb::None).boxed()),
        (5, n.clone().map(Bomb::Ord).boxed()),
        (3, n.clone().map(Bomb::Ik).boxed()),
        (3, n.clone().map(Bomb::Drop).boxed()),
        (2, n.map(Bomb::Closure).boxed()),
    ])
    .boxed()
}

fn kp() -> BoxGen<KP> {
    (gens::frequency(vec![(3, gens::constant(true).boxed()), (2, gens::constant(false).boxed())]), gens::int(0u32..=u32::MAX))
        .map(|(near, x)| KP { near, x })
        .boxed()
}

fn op() -> BoxGen<Op> {
    let v = gens::int(0u32..=u32::MAX);
    let len = gens::frequency(vec![(3, gens::int(0u16..=20).boxed()), (2, gens::int(0u16..=400).boxed()), (1, gens::int(0u16..=3000).boxed())]);
    let stride = gens::frequency(vec![(3, gens::int(1u8..=3).boxed()), (1, gens::int(1u8..=255).boxed())]);
    let small = gens::int(0u8..=255);
    gens::frequency(vec![
        (6, (kp(), v.clone(), bomb()).map(|(k, v, bomb)| Op::Insert { k, v, bomb }).boxed()),
        (5, (kp(), gens::bool(), bomb()).map(|(k, entry, bomb)| Op::Remove { k, entry, bomb }).boxed()),
        (3, (kp(), gens::int(0u8..=3), v.clone(), bomb()).map(|(k, how, v, bomb)| Op::Entry { k, how, v, bomb }).boxed()),
        (2, (gens::bool(), bomb()).map(|(last, bomb)| Op::Pop { last, bomb }).boxed()),
        (2, (kp(), stride.clone(), len.clone(), bomb()).map(|(start, stride, len, bomb)| Op::Append { start, stride, len, bomb }).boxed()),
        (2, (kp(), gens::bool(), bomb()).map(|(k, upper, bomb)| Op::SplitOff { k, upper, bomb }).boxed()),
        (
            2,
            (kp(), gens::option(kp()), gens::int(1u8..=6), small.clone(), len.clone(), gens::bool(), bomb())
                .map(|(lo, hi, m, r, take, bump, bomb)| Op::ExtractIf { lo, hi, m, r: r % m, take, bump, bomb })
                .boxed(),
        ),
        (2, (gens::int(1u8..=6), small, gens::bool(), bomb()).map(|(m, r, bump, bomb)| Op::Retain { m, r: r % m, bump, bomb }).boxed()),
        (2, (gens::vec(kp()).len(8..=8), v, bomb()).map(|(keys, v, bomb)| Op::InsertMany { keys, v, bomb }).boxed()),
        (2, (kp(), stride.clone(), len.clone(), gens::bool(), bomb()).map(|(start, stride, len, down, bomb)| Op::Extend { start, stride, len, down, bomb }).boxed()),
        (1, bomb().map(|bomb| Op::Clear { bomb }).boxed()),
        (1, bomb().map(|bomb| Op::Drop { bomb }).boxed()),
        (1, (gens::int(0u16..=300), gens::int(0u16..=300), bomb()).map(|(front, back, bomb)| Op::IntoIter { front, back, bomb }).boxed()),
        (1, (kp(), stride, len, bomb()).map(|(start, stride, len, bomb)| Op::FromIter { start, stride, len, bomb }).boxed()),
        (4, (gens::int(0u8..=3), kp(), bomb()).map(|(which, k, bomb)| Op::Set { which, k, bomb }).boxed()),
        (
            2,
            (gens::int(0u8..=2), kp(), gens::frequency(vec![(3, gens::int(0u16..=40).boxed()), (1, gens::int(0u16..=2000).boxed())]), gens::int(1u8..=5), bomb())
                .map(|(which, k, len, m, bomb)| Op::SetBulk { which, k, len, m, bomb })
                .boxed(),
        ),
    ])
    .boxed()
}

pub fn case() -> BoxGen<Case> {
    (
        gens::int(0u8..=2),
        gens::int(0u32..=5_000),
        gens::frequency(vec![(4, gens::int(1u16..=3).boxed()), (1, gens::int(1u16..=1_000).boxed())]),
        map_len().map(|n| n.min(12_000) as u16),
        gens::frequency(vec![(3, gens::int(0u16..=40).boxed()), (2, gens::int(0u16..=2_000).boxed())]),
        gens::vec(op()).len(1..=24),
    )
        .map(|(mode, start, stride, len, set_len, ops)| Case { mode, start, stride, len, set_len, ops })
        .boxed()
}

type Map<K, S> = BBTreeMap<K, TV, S, Pages>;
type Set<K, S> = BBTreeSet<K, S, Pages>;
type Model = BTreeMap<u64, u64>;

fn arm(b: Bomb) {
    FIRED.with(|f| f.set(0));
    match b {
        Bomb::None => {}
        Bomb::Ord(n) => ORD.with(|c| c.set(u32::from(n))),
        Bomb::Ik(n) => IK.with(|c| c.set(u32::from(n))),
        Bomb::Drop(n) => DROPS.with(|c| c.set(u32::from(n))),
        Bomb::Closure(n) => CLOSURE.with(|c| c.set(u32::from(n))),
    }
}

/// Disarms every bomb; returns which went off.
fn disarm() -> u8 {
    for c in [&ORD, &IK, &DROPS, &CLOSURE] {
        c.with(|c| c.set(0));
    }
    FIRED.with(|f| f.get())
}

fn panic_msg(p: &(dyn std::any::Any + Send)) -> String {
    p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_else(|| "?".into())
}

/// Runs an op with its bomb armed. `Ok(fired)` (0: none fired, nothing
/// panicked); a panic no bomb made is a failure.
fn armed(name: &str, b: Bomb, f: impl FnOnce()) -> Result<u8, Fail> {
    arm(b);
    let r = catch_unwind(AssertUnwindSafe(f));
    let fired = disarm();
    LAST.with(|l| l.set(fired));
    match r {
        Ok(()) => Ok(0),
        Err(p) if fired != 0 => {
            let _ = p;
            Ok(fired)
        }
        Err(p) => Err(fail(&format!("upanic__panic__{name}"), format!("panicked with no bomb gone off: {}", panic_msg(&*p)))),
    }
}

fn snap<K: PKey, S: InlineKeyStrategy<K>>(map: &Map<K, S>) -> Vec<(u64, u64)> {
    map.iter().map(|(k, v)| (k.n(), v.n)).collect()
}

fn as_vec(m: &Model) -> Vec<(u64, u64)> {
    m.iter().map(|(&k, &v)| (k, v)).collect()
}

fn first_diff(a: &[(u64, u64)], b: &[(u64, u64)]) -> String {
    let p = a.iter().zip(b).position(|(x, y)| x != y).unwrap_or(a.len().min(b.len()));
    format!("len {} vs {}; at {p}: {:?} vs {:?}", a.len(), b.len(), a.get(p), b.get(p))
}

struct Ctx {
    mode: u8,
    span: u64,
}

impl Ctx {
    fn key(&self, kp: KP, model: &Model) -> u64 {
        if kp.near && !model.is_empty() {
            let base = *model.keys().nth(kp.x as usize % model.len()).unwrap();
            (base as i64 + i64::from((kp.x >> 24) % 3) - 1).max(0) as u64
        } else {
            u64::from(kp.x) % self.span
        }
    }
}

/// The op's post-state must be one of `allowed` (labelled for the message).
fn one_of(name: &str, fired: u8, got: &[(u64, u64)], allowed: &[(&str, &Model)]) -> Result<(), Fail> {
    if allowed.iter().any(|(_, m)| m.len() == got.len() && m.iter().zip(got).all(|((&k, &v), &(gk, gv))| k == gk && v == gv)) {
        return Ok(());
    }
    let kind = if fired == 0 { "mismatch" } else { "state" };
    let (label, m) = allowed[0];
    Err(fail(
        &format!("upanic__{kind}__{name}"),
        format!("bomb fired: {fired}; not one of {:?}; vs {label}: {}", allowed.iter().map(|a| a.0).collect::<Vec<_>>(), first_diff(got, &as_vec(m))),
    ))
}

/// The tree's own checks on a map the ledger doesn't cover.
#[allow(unused_variables)]
fn tree_checks<K: PKey, S: InlineKeyStrategy<K>>(name: &str, map: &Map<K, S>) -> Result<(), Fail> {
    #[cfg(fuzzing)]
    if let Err(p) = catch_unwind(AssertUnwindSafe(|| {
        map.check_invariants();
        map.check_prefix_lens();
    })) {
        return Err(fail(&format!("upanic__invariant__{name}"), panic_msg(&*p)));
    }
    ensure(map.iter().count() == map.len(), &format!("upanic__len__{name}"), || format!("len {} walk {}", map.len(), map.iter().count()))
}

/// Checks after every op: the tree's own checks, len against the walk, the ledger.
///
/// After an `InlineKey` panic only `check_prefix_lens` ("it holds even for
/// keys that panic"): a refit it stops "may leave a node's inline keys off
/// its prefix, which only makes searches wrong" (upstream's
/// `a_panicking_inline_key_leaves_a_sound_map`), so the run rebuilds the
/// map after one. After a `Drop` panic outside a teardown, what the unwind
/// leaked is forgotten: Rust leaks a return value whose destructor-panicking
/// neighbour unwinds (std's `remove` too); teardowns must drop everything.
fn after<K: PKey, S: InlineKeyStrategy<K>>(name: &str, map: &Map<K, S>, set: &Set<K, S>) -> Result<(), Fail> {
    let fired = LAST.with(|l| l.replace(0));
    #[cfg(fuzzing)]
    {
        let r = catch_unwind(AssertUnwindSafe(|| {
            if fired != 2 {
                map.check_invariants();
                set.check_invariants();
            }
            map.check_prefix_lens();
        }));
        if let Err(p) = r {
            return Err(fail(&format!("upanic__invariant__{name}"), panic_msg(&*p)));
        }
    }
    ensure(map.iter().count() == map.len() && set.iter().count() == set.len(), &format!("upanic__len__{name}"), || {
        format!("map len {} walk {}; set len {} walk {}", map.len(), map.iter().count(), set.len(), set.iter().count())
    })?;
    let double = DOUBLE.with(|d| d.replace(0));
    ensure(double == 0, &format!("upanic__double_drop__{name}"), || format!("{double} keys or values dropped twice"))?;
    let mut ids: HashSet<u64> = map.iter().flat_map(|(k, v)| [k.id(), v.id]).collect();
    ids.extend(set.iter().map(|k| k.id()));
    if fired == 3 && !matches!(name, "clear" | "drop" | "into_iter") {
        LIVE.with(|l| l.borrow_mut().retain(|id| ids.contains(id)));
    }
    let live = LIVE.with(|l| l.borrow().clone());
    ensure(live == ids, &format!("upanic__leak__{name}"), || {
        format!("{} alive, {} held by the map and set; {} alive not held, {} held not alive", live.len(), ids.len(), live.difference(&ids).count(), ids.difference(&live).count())
    })
}

/// The drop log of a teardown must be `expect` (ascending, each key before its value).
fn drop_order(name: &str, log: Vec<(bool, u64)>, expect: &[(u64, u64)]) -> Result<(), Fail> {
    let want: Vec<(bool, u64)> = expect.iter().flat_map(|&(k, v)| [(false, k), (true, v)]).collect();
    let p = log.iter().zip(&want).position(|(a, b)| a != b);
    ensure(log == want, &format!("upanic__drop_order__{name}"), || {
        let p = p.unwrap_or(log.len().min(want.len()));
        format!("{} drops, {} expected; at {p}: {:?} vs {:?}", log.len(), want.len(), log.get(p), want.get(p))
    })
}

/// A bulk change processed element by element in key order (retain,
/// extract_if, a panicking one stops part way): each stored entry, in
/// order, must be processed, then at most one bumped-but-kept, then
/// untouched.
fn prefix_split(name: &str, fired: u8, before: &Model, done: &Model, bumped: &Model, got: &[(u64, u64)]) -> Result<(), Fail> {
    let gm: Model = got.iter().copied().collect();
    let bad = |why: String| Err(fail(&format!("upanic__state__{name}"), format!("bomb fired: {fired}; {why}")));
    if let Some(k) = gm.keys().find(|k| !before.contains_key(k)) {
        return bad(format!("key {k} was not in the map"));
    }
    // 0: processing, 1: past the stop
    let mut phase = 0;
    for (&k, &v) in before {
        let now = gm.get(&k).copied();
        let is_done = now == done.get(&k).copied();
        let is_bumped = now == bumped.get(&k).copied();
        let is_untouched = now == Some(v);
        if phase == 0 {
            if is_done {
                continue;
            }
            phase = 1;
            if is_bumped || is_untouched {
                continue;
            }
            return bad(format!("key {k}: {now:?}, neither done ({:?}), bumped nor untouched ({v})", done.get(&k)));
        } else if !is_untouched {
            return bad(format!("key {k}: {now:?} past the stop, not untouched ({v})"));
        }
    }
    Ok(())
}

pub fn run<K: PKey, S: InlineKeyStrategy<K> + Default>(case: &Case) -> Result<(), Fail> {
    disarm();
    DOUBLE.with(|d| d.set(0));
    LIVE.with(|l| l.borrow_mut().clear());
    LOG.with(|l| *l.borrow_mut() = None);
    let p0 = pages();
    let r = run_inner::<K, S>(case);
    disarm();
    LOG.with(|l| *l.borrow_mut() = None);
    r?;
    let live = LIVE.with(|l| l.borrow().len());
    ensure(live == 0, "upanic__leak__end", || format!("{live} keys or values alive after everything dropped"))?;
    ensure(DOUBLE.with(|d| d.get()) == 0, "upanic__double_drop__end", || "dropped twice at the end".into())?;
    ensure(pages() == p0, "upanic__leak__pages", || format!("{} pages out after everything dropped", pages() - p0))
}

fn new_map<K: PKey, S: InlineKeyStrategy<K> + Default>() -> Map<K, S> {
    BBTreeMap::with_strategy_in(S::default(), Pages)
}

fn run_inner<K: PKey, S: InlineKeyStrategy<K> + Default>(case: &Case) -> Result<(), Fail> {
    let mode = case.mode;
    let stride = u64::from(case.stride);
    let ctx = Ctx { mode, span: u64::from(case.start) + u64::from(case.len) * stride + 64 };
    let mut map: Map<K, S> = new_map();
    let mut model = Model::new();
    for i in 0..u64::from(case.len) {
        let n = u64::from(case.start) + i * stride;
        let v = n.wrapping_mul(0x9E37_79B9) >> 34;
        map.insert(K::make(n, mode), TV::new(v));
        model.insert(n, v);
    }
    let mut set: Set<K, S> = BBTreeSet::with_strategy_in(S::default(), Pages);
    let mut smodel: BTreeSet<u64> = BTreeSet::new();
    for i in 0..u64::from(case.set_len) {
        let n = u64::from(case.start) + i * (stride + 1);
        set.insert(K::make(n, mode));
        smodel.insert(n);
    }
    after("load", &map, &set)?;
    let mut fired_any = false;
    for (i, op) in case.ops.iter().enumerate() {
        LAST.with(|l| l.set(0));
        let name = step(&ctx, op, &mut map, &mut model, &mut set, &mut smodel, &mut fired_any).map_err(|f| Fail { sig: f.sig, msg: format!("op {i} {op:?}: {}", f.msg) })?;
        let ik = LAST.with(|l| l.get()) == 2;
        after(name, &map, &set).map_err(|f| Fail { sig: f.sig, msg: format!("op {i} {op:?}: {}", f.msg) })?;
        if ik {
            // searches of a map an InlineKey panic left may be wrong: go on with the same entries in a new one
            for (k, v) in std::mem::replace(&mut map, new_map()) {
                map.insert(k, v);
            }
            for k in std::mem::replace(&mut set, BBTreeSet::with_strategy_in(S::default(), Pages)) {
                set.insert(k);
            }
        }
    }
    classify(fired_any, "bomb_fired");
    classify(model.len() > 1000, "map_over_1000");
    drop(set);
    // the final drop, logged and checked like the Drop op's
    let expect = as_vec(&model);
    LOG.with(|l| *l.borrow_mut() = Some(Vec::new()));
    drop(map);
    let log = LOG.with(|l| l.borrow_mut().take()).unwrap_or_default();
    drop_order("end", log, &expect)
}

fn step<K: PKey, S: InlineKeyStrategy<K> + Default>(
    ctx: &Ctx,
    op: &Op,
    map: &mut Map<K, S>,
    model: &mut Model,
    set: &mut Set<K, S>,
    smodel: &mut BTreeSet<u64>,
    fired_any: &mut bool,
) -> Result<&'static str, Fail> {
    let mode = ctx.mode;
    let before = model.clone();
    let mut note = |f: u8| {
        if f != 0 {
            *fired_any = true;
        }
    };
    match *op {
        Op::Insert { k, v, bomb } => {
            let name = "insert";
            let n = ctx.key(k, model);
            let v = u64::from(v);
            let mut want = before.clone();
            let old = want.insert(n, v);
            let got = RefCell::new(None);
            let fired = armed(name, bomb, || {
                let r = map.insert(K::make(n, mode), TV::new(v));
                *got.borrow_mut() = Some(r.as_ref().map(|t| t.n));
            })?;
            note(fired);
            let s = snap(map);
            match fired {
                0 => {
                    one_of(name, 0, &s, &[("after", &want)])?;
                    let r = got.into_inner().unwrap();
                    ensure(r == old, "upanic__mismatch__insert", || format!("returned {r:?}, std {old:?}"))?;
                }
                1 => one_of(name, 1, &s, &[("unchanged", &before)])?,
                _ => one_of(name, fired, &s, &[("unchanged", &before), ("after", &want)])?,
            }
            *model = s.into_iter().collect();
            Ok(name)
        }
        Op::Remove { k, entry, bomb } => {
            let name = "remove";
            let n = ctx.key(k, model);
            let mut want = before.clone();
            let old = want.remove(&n);
            let got = RefCell::new(None);
            let q = K::make(n, mode);
            let fired = armed(name, bomb, || {
                let r = if entry { map.remove_entry(&q).map(|(k, v)| (k.n(), v.n)) } else { map.remove(&q).map(|v| (n, v.n)) };
                *got.borrow_mut() = Some(r);
            })?;
            drop(q);
            note(fired);
            let s = snap(map);
            match fired {
                0 => {
                    one_of(name, 0, &s, &[("after", &want)])?;
                    let r = got.into_inner().unwrap();
                    ensure(r == old.map(|v| (n, v)), "upanic__mismatch__remove", || format!("returned {r:?}, std {old:?}"))?;
                }
                1 => one_of(name, 1, &s, &[("unchanged", &before)])?,
                _ => one_of(name, fired, &s, &[("unchanged", &before), ("after", &want)])?,
            }
            *model = s.into_iter().collect();
            Ok(name)
        }
        Op::Entry { k, how, v, bomb } => {
            let name = "entry";
            let n = ctx.key(k, model);
            let v = u64::from(v);
            let mut want = before.clone();
            match (how, want.get(&n).copied()) {
                (0, None) | (3, None) => {
                    want.insert(n, v);
                }
                (1, Some(old)) => {
                    want.insert(n, old.wrapping_add(1));
                }
                (2, Some(_)) => {
                    want.remove(&n);
                }
                (3, Some(_)) => {
                    want.insert(n, v);
                }
                _ => {}
            }
            let fired = armed(name, bomb, || {
                use bbtree::unsync::bbtree_map::Entry;
                let e = map.entry(K::make(n, mode));
                match (how, e) {
                    (0, e) => {
                        e.or_insert(TV::new(v));
                    }
                    (1, e) => {
                        e.and_modify(|t| t.n = t.n.wrapping_add(1));
                    }
                    (2, Entry::Occupied(o)) => drop(o.remove_entry()),
                    (3, Entry::Occupied(mut o)) => drop(o.insert(TV::new(v))),
                    (3, Entry::Vacant(e)) => {
                        e.insert(TV::new(v));
                    }
                    _ => {}
                }
            })?;
            note(fired);
            let s = snap(map);
            match fired {
                0 => one_of(name, 0, &s, &[("after", &want)])?,
                _ => one_of(name, fired, &s, &[("unchanged", &before), ("after", &want)])?,
            }
            // an Ord panic happens in `entry` itself, before any change
            if fired == 1 {
                one_of(name, 1, &s, &[("unchanged", &before)])?;
            }
            *model = s.into_iter().collect();
            Ok(name)
        }
        Op::Pop { last, bomb } => {
            let name = "pop";
            let mut want = before.clone();
            let old = if last { want.pop_last() } else { want.pop_first() };
            let got = RefCell::new(None);
            let fired = armed(name, bomb, || {
                let r = if last { map.pop_last() } else { map.pop_first() };
                *got.borrow_mut() = Some(r.map(|(k, v)| (k.n(), v.n)));
            })?;
            note(fired);
            let s = snap(map);
            if fired == 0 {
                let r = got.into_inner().unwrap();
                ensure(r == old, "upanic__mismatch__pop", || format!("returned {r:?}, std {old:?}"))?;
                one_of(name, 0, &s, &[("after", &want)])?;
            } else {
                one_of(name, fired, &s, &[("unchanged", &before), ("after", &want)])?;
            }
            *model = s.into_iter().collect();
            Ok(name)
        }
        Op::Append { start, stride, len, bomb } => {
            let name = "append";
            let s0 = ctx.key(start, model);
            // values no earlier append gave: a merged one shows
            let salt = NEXT_ID.with(|n| n.get()) & 0x7fff_ffff;
            let mut other: Map<K, S> = new_map();
            let mut omodel = Model::new();
            for i in 0..u64::from(len) {
                let n = s0 + i * u64::from(stride);
                let v = (1 << 63) | (salt << 32) | (n & 0xffff_ffff);
                other.insert(K::make(n, mode), TV::new(v));
                omodel.insert(n, v);
            }
            let mut want = before.clone();
            want.extend(omodel.iter().map(|(&k, &v)| (k, v)));
            let fired = armed(name, bomb, || map.append(&mut other))?;
            note(fired);
            let s = snap(map);
            let olen = other.iter().count();
            if fired == 0 {
                one_of(name, 0, &s, &[("after", &want)])?;
                ensure(olen == 0 && other.is_empty(), "upanic__mismatch__append", || format!("other keeps {olen}"))?;
            } else if fired == 1 {
                ensure(olen == 0 && other.is_empty(), "upanic__state__append", || format!("Ord panicked; other keeps {olen} (len {}), the rest should be dropped", other.len()))?;
                let gm: Model = s.iter().copied().collect();
                let bad = |why: String| Err(fail("upanic__state__append", format!("Ord panicked: {why}")));
                if let Some(k) = before.keys().find(|k| !gm.contains_key(k)) {
                    return bad(format!("own key {k} lost"));
                }
                let mut stopped = None;
                for (&k, &v) in &omodel {
                    let merged = gm.get(&k) == Some(&v);
                    match (merged, stopped) {
                        (false, None) => stopped = Some(k),
                        (true, Some(at)) => return bad(format!("other's key {k} merged, but {at} before it not")),
                        _ => {}
                    }
                }
                for (&k, &v) in &gm {
                    let ok = before.get(&k) == Some(&v) || omodel.get(&k) == Some(&v);
                    if !ok {
                        return bad(format!("key {k} -> {v:#x} from neither map"));
                    }
                }
            }
            drop(other);
            *model = s.into_iter().collect();
            Ok(name)
        }
        Op::SplitOff { k, upper, bomb } => {
            let name = "split_off";
            let n = ctx.key(k, model);
            let mut lower = before.clone();
            let up = lower.split_off(&n);
            let q = K::make(n, mode);
            let got: RefCell<Option<Map<K, S>>> = RefCell::new(None);
            let fired = armed(name, bomb, || *got.borrow_mut() = Some(map.split_off(&q)))?;
            drop(q);
            note(fired);
            let s = snap(map);
            match fired {
                0 => {
                    one_of(name, 0, &s, &[("lower", &lower)])?;
                    let hi = got.into_inner().unwrap();
                    one_of(name, 0, &snap(&hi), &[("upper", &up)])?;
                    tree_checks(name, &hi)?;
                    if upper {
                        *map = hi;
                        *model = up;
                    } else {
                        drop(hi);
                        *model = lower;
                    }
                    return Ok(name);
                }
                1 => one_of(name, 1, &s, &[("unchanged", &before)])?,
                _ => one_of(name, fired, &s, &[("unchanged", &before), ("lower", &lower)])?,
            }
            *model = s.into_iter().collect();
            Ok(name)
        }
        Op::ExtractIf { lo, hi, m, r, take, bump, bomb } => {
            let name = "extract_if";
            let lo = ctx.key(lo, model);
            let hi = hi.map(|h| ctx.key(h, model).max(lo));
            let in_range = |k: u64| k >= lo && hi.is_none_or(|h| k < h);
            let pred = |k: u64| (k + u64::from(r)) % u64::from(m) == 0;
            // the model: visit in order until `take` yielded or the closure bomb's call
            let (mut done, mut bumped, mut yielded_want) = (before.clone(), before.clone(), Vec::new());
            let mut calls = 0u32;
            let stop_at = if let Bomb::Closure(c) = bomb { u32::from(c) } else { u32::MAX };
            for (&k, &v) in before.iter().filter(|(k, _)| in_range(**k)) {
                if yielded_want.len() >= usize::from(take) {
                    break;
                }
                calls += 1;
                let nv = if bump { v.wrapping_add(1) } else { v };
                bumped.insert(k, nv);
                if calls == stop_at {
                    done.insert(k, nv);
                    break;
                }
                if pred(k) {
                    done.remove(&k);
                    yielded_want.push((k, nv));
                } else {
                    done.insert(k, nv);
                }
            }
            let yielded: RefCell<Vec<(K, TV)>> = RefCell::new(Vec::new());
            let fired = armed(name, bomb, || {
                let range = (Bound::Included(K::make(lo, mode)), hi.map_or(Bound::Unbounded, |h| Bound::Excluded(K::make(h, mode))));
                let it = map.extract_if(range, |k, v| {
                    if bump {
                        v.n = v.n.wrapping_add(1);
                    }
                    tick(&CLOSURE, 4, "an extract_if closure");
                    pred(k.n())
                });
                for e in it.take(usize::from(take)) {
                    yielded.borrow_mut().push(e);
                }
            })?;
            note(fired);
            let s = snap(map);
            let ys: Vec<(u64, u64)> = yielded.borrow().iter().map(|(k, v)| (k.n(), v.n)).collect();
            drop(yielded);
            if fired == 0 || fired == 4 {
                one_of(name, fired, &s, &[("after", &done)])?;
                ensure(ys == yielded_want, &format!("upanic__{}__extract_if", if fired == 0 { "mismatch" } else { "state" }), || {
                    format!("yielded {} {:?}..., std {} {:?}...", ys.len(), ys.first(), yielded_want.len(), yielded_want.first())
                })?;
            } else {
                prefix_split(name, fired, &before, &done, &bumped, &s)?;
                // every entry gone was yielded, but for one an InlineKey panic took out
                let gm: BTreeSet<u64> = s.iter().map(|e| e.0).collect();
                let gone = before.keys().filter(|k| !gm.contains(k)).count();
                let slack = usize::from(fired == 2);
                ensure(gone >= ys.len() && gone <= ys.len() + slack, "upanic__state__extract_if", || format!("bomb fired: {fired}; {gone} gone, {} yielded", ys.len()))?;
            }
            *model = s.into_iter().collect();
            Ok(name)
        }
        Op::Retain { m, r, bump, bomb } => {
            let name = "retain";
            let pred = |k: u64| (k + u64::from(r)) % u64::from(m) == 0;
            let (mut done, mut bumped) = (before.clone(), before.clone());
            let stop_at = if let Bomb::Closure(c) = bomb { u32::from(c) } else { u32::MAX };
            for (calls, (&k, &v)) in (1u32..).zip(before.iter()) {
                let nv = if bump { v.wrapping_add(1) } else { v };
                bumped.insert(k, nv);
                if calls == stop_at {
                    done.insert(k, nv);
                    break;
                }
                if pred(k) {
                    done.remove(&k);
                } else {
                    done.insert(k, nv);
                }
            }
            let fired = armed(name, bomb, || {
                map.retain(|k, v| {
                    if bump {
                        v.n = v.n.wrapping_add(1);
                    }
                    tick(&CLOSURE, 4, "a retain closure");
                    !pred(k.n())
                })
            })?;
            note(fired);
            let s = snap(map);
            if fired == 0 || fired == 4 {
                one_of(name, fired, &s, &[("after", &done)])?;
            } else {
                prefix_split(name, fired, &before, &done, &bumped, &s)?;
            }
            *model = s.into_iter().collect();
            Ok(name)
        }
        Op::InsertMany { ref keys, v, bomb } => {
            let name = "insert_many";
            let ns: Vec<u64> = keys.iter().map(|&k| ctx.key(k, model)).collect();
            let vs: Vec<u64> = (0..8).map(|i| u64::from(v).wrapping_add(i)).collect();
            let mut want = before.clone();
            let olds: Vec<Option<u64>> = ns.iter().zip(&vs).map(|(&n, &v)| want.insert(n, v)).collect();
            let got = RefCell::new(None);
            let fired = armed(name, bomb, || {
                let e: [(K, TV); 8] = std::array::from_fn(|i| (K::make(ns[i], mode), TV::new(vs[i])));
                let r = map.insert_many(e);
                *got.borrow_mut() = Some(r.iter().map(|o| o.as_ref().map(|t| t.n)).collect::<Vec<_>>());
            })?;
            note(fired);
            let s = snap(map);
            if fired == 0 {
                one_of(name, 0, &s, &[("after", &want)])?;
                let r = got.into_inner().unwrap();
                ensure(r == olds, "upanic__mismatch__insert_many", || format!("returned {r:?}, std {olds:?}"))?;
            } else {
                // "entries not yet inserted are dropped": each key holds its old value or one the batch gave it
                let gm: Model = s.iter().copied().collect();
                for (&k, &v) in &before {
                    let ok = gm.get(&k) == Some(&v) || ns.iter().zip(&vs).any(|(&n, &bv)| n == k && gm.get(&k) == Some(&bv));
                    ensure(ok, "upanic__state__insert_many", || format!("bomb fired: {fired}; key {k} holds {:?}, had {v}", gm.get(&k)))?;
                }
                for (&k, &v) in &gm {
                    let ok = before.get(&k) == Some(&v) || ns.iter().zip(&vs).any(|(&n, &bv)| n == k && bv == v);
                    ensure(ok, "upanic__state__insert_many", || format!("bomb fired: {fired}; key {k} -> {v} from nowhere"))?;
                }
            }
            *model = s.into_iter().collect();
            Ok(name)
        }
        Op::Extend { start, stride, len, down, bomb } => {
            let name = "extend";
            let s0 = ctx.key(start, model);
            let mut ns: Vec<u64> = (0..u64::from(len)).map(|i| s0 + i * u64::from(stride)).collect();
            if down {
                ns.reverse();
            }
            let vs: Vec<u64> = ns.iter().map(|n| n ^ 0x5555).collect();
            let stop_at = if let Bomb::Closure(c) = bomb { usize::from(c) } else { usize::MAX };
            let fired = armed(name, bomb, || {
                let it = ns.iter().zip(&vs).enumerate().map(|(i, (&n, &v))| {
                    if i + 1 == stop_at {
                        tick(&CLOSURE, 4, "an extend iterator");
                    }
                    (K::make(n, mode), TV::new(v))
                });
                if let Bomb::Closure(c) = bomb {
                    CLOSURE.with(|x| x.set(u32::from(c.min(1))));
                }
                map.extend(it);
            })?;
            note(fired);
            let s = snap(map);
            // the states after each prefix of the entries
            let pulled = if fired == 4 { stop_at - 1 } else { ns.len() };
            let lo = if fired == 0 { ns.len() } else if fired == 4 { pulled.saturating_sub(16) } else { 0 };
            let mut st = before.clone();
            for (&n, &v) in ns.iter().zip(&vs).take(lo) {
                st.insert(n, v);
            }
            let mut ok = one_of(name, fired, &s, &[("after", &st)]).is_ok();
            let mut j = lo;
            while !ok && fired != 0 && j < pulled.min(ns.len()) {
                st.insert(ns[j], vs[j]);
                j += 1;
                ok = one_of(name, fired, &s, &[("after", &st)]).is_ok();
            }
            if !ok && fired != 4 && fired != 0 {
                // Ord / InlineKey / Drop: inside a batch, keys may land out of input order; per key only
                let gm: Model = s.iter().copied().collect();
                ok = before.iter().all(|(k, v)| gm.contains_key(k) && (gm[k] == *v || ns.iter().zip(&vs).any(|(n, bv)| n == k && gm[k] == *bv)))
                    && gm.iter().all(|(k, v)| before.get(k) == Some(v) || ns.iter().zip(&vs).any(|(n, bv)| n == k && bv == v));
            }
            if !ok {
                let mut full = before.clone();
                full.extend(ns.iter().copied().zip(vs.iter().copied()));
                return Err(fail(
                    &format!("upanic__{}__extend", if fired == 0 { "mismatch" } else { "state" }),
                    format!("bomb fired: {fired}; not the state after any of entries {lo}..={pulled} of {}: vs all: {}", ns.len(), first_diff(&s, &as_vec(&full))),
                ));
            }
            *model = s.into_iter().collect();
            Ok(name)
        }
        Op::Clear { bomb } | Op::Drop { bomb } => {
            let clear = matches!(op, Op::Clear { .. });
            let name = if clear { "clear" } else { "drop" };
            let expect = as_vec(&before);
            LOG.with(|l| *l.borrow_mut() = Some(Vec::new()));
            let fired = armed(name, bomb, || {
                if clear {
                    map.clear();
                } else {
                    drop(std::mem::replace(map, new_map()));
                }
            })?;
            note(fired);
            let log = LOG.with(|l| l.borrow_mut().take()).unwrap_or_default();
            drop_order(name, log, &expect)?;
            ensure(map.is_empty() && map.iter().next().is_none(), &format!("upanic__state__{name}"), || format!("bomb fired: {fired}; {} entries left", map.len()))?;
            model.clear();
            Ok(name)
        }
        Op::IntoIter { front, back, bomb } => {
            let name = "into_iter";
            let all = as_vec(&before);
            let f = usize::from(front).min(all.len());
            let b = usize::from(back).min(all.len() - f);
            let taken: RefCell<Vec<(K, TV)>> = RefCell::new(Vec::new());
            let whole = std::mem::replace(map, new_map());
            model.clear();
            let mut it = whole.into_iter();
            for _ in 0..f {
                taken.borrow_mut().extend(it.next());
            }
            for _ in 0..b {
                taken.borrow_mut().extend(it.next_back());
            }
            let ends: Vec<(u64, u64)> = taken.borrow().iter().map(|(k, v)| (k.n(), v.n)).collect();
            let want_ends: Vec<(u64, u64)> = all[..f].iter().chain(all[all.len() - b..].iter().rev()).copied().collect();
            ensure(ends == want_ends, "upanic__mismatch__into_iter", || format!("ends {} {:?}..., std {} {:?}...", ends.len(), ends.first(), want_ends.len(), want_ends.first()))?;
            drop(taken);
            LOG.with(|l| *l.borrow_mut() = Some(Vec::new()));
            let fired = armed(name, bomb, || drop(it))?;
            note(fired);
            let log = LOG.with(|l| l.borrow_mut().take()).unwrap_or_default();
            drop_order(name, log, &all[f..all.len() - b])?;
            Ok(name)
        }
        Op::FromIter { start, stride, len, bomb } => {
            let name = "from_iter";
            let s0 = ctx.key(start, model);
            let ns: Vec<u64> = (0..u64::from(len)).map(|i| s0 + (i * u64::from(stride)) % (u64::from(len) * 2 + 1)).collect();
            let want: Model = ns.iter().map(|&n| (n, n ^ 0xAAAA)).collect();
            let got = RefCell::new(None);
            let fired = armed(name, bomb, || {
                let m: BBTreeMap<K, TV> = ns.iter().map(|&n| (K::make(n, mode), TV::new(n ^ 0xAAAA))).collect();
                *got.borrow_mut() = Some(m.iter().map(|(k, v)| (k.n(), v.n)).collect::<Vec<_>>());
                #[cfg(fuzzing)]
                m.check_invariants();
            })?;
            note(fired);
            if fired == 0 {
                let g = got.into_inner().unwrap();
                one_of(name, 0, &g, &[("std", &want)])?;
            }
            Ok(name)
        }
        Op::SetBulk { which, k, len, m, bomb } => {
            let name = ["set_append", "set_split_off", "set_retain"][usize::from(which % 3)];
            let n = ctx.key(k, model);
            let before = smodel.clone();
            let as_m = |s: &BTreeSet<u64>| s.iter().map(|&k| (k, 0)).collect::<Model>();
            let snap_set = |set: &Set<K, S>| set.iter().map(|k| (k.n(), 0)).collect::<Vec<_>>();
            let fired;
            match which % 3 {
                0 => {
                    let step = u64::from(m % 3) + 1;
                    let mut other: Set<K, S> = BBTreeSet::with_strategy_in(S::default(), Pages);
                    let omodel: BTreeSet<u64> = (0..u64::from(len)).map(|i| n + i * step).collect();
                    for &x in &omodel {
                        other.insert(K::make(x, mode));
                    }
                    let mut want = before.clone();
                    want.extend(omodel.iter().copied());
                    fired = armed(name, bomb, || set.append(&mut other))?;
                    let s = snap_set(set);
                    let olen = other.iter().count();
                    if fired == 0 {
                        one_of(name, 0, &s, &[("after", &as_m(&want))])?;
                        ensure(olen == 0, "upanic__mismatch__set_append", || format!("other keeps {olen}"))?;
                    } else if fired == 1 {
                        let got: BTreeSet<u64> = s.iter().map(|e| e.0).collect();
                        let bad = |why: String| Err(fail("upanic__state__set_append", format!("Ord panicked: {why}")));
                        if olen != 0 {
                            return bad(format!("other keeps {olen}"));
                        }
                        if let Some(x) = before.iter().find(|x| !got.contains(x)) {
                            return bad(format!("own key {x} lost"));
                        }
                        if let Some(x) = got.iter().find(|x| !before.contains(x) && !omodel.contains(x)) {
                            return bad(format!("key {x} from neither set"));
                        }
                        let mut stopped = None;
                        for x in omodel.iter().filter(|x| !before.contains(x)) {
                            match (got.contains(x), stopped) {
                                (false, None) => stopped = Some(*x),
                                (true, Some(at)) => return bad(format!("other's key {x} merged, but {at} before it not")),
                                _ => {}
                            }
                        }
                    }
                    drop(other);
                    *smodel = s.into_iter().map(|e| e.0).collect();
                }
                1 => {
                    let mut lower = before.clone();
                    let up = lower.split_off(&n);
                    let q = K::make(n, mode);
                    let got: RefCell<Option<Set<K, S>>> = RefCell::new(None);
                    fired = armed(name, bomb, || *got.borrow_mut() = Some(set.split_off(&q)))?;
                    drop(q);
                    let s = snap_set(set);
                    match fired {
                        0 => {
                            one_of(name, 0, &s, &[("lower", &as_m(&lower))])?;
                            let hi = got.into_inner().unwrap();
                            one_of(name, 0, &snap_set(&hi), &[("upper", &as_m(&up))])?;
                            #[cfg(fuzzing)]
                            if let Err(p) = catch_unwind(AssertUnwindSafe(|| hi.check_invariants())) {
                                return Err(fail("upanic__invariant__set_split_off", panic_msg(&*p)));
                            }
                        }
                        1 => one_of(name, 1, &s, &[("unchanged", &as_m(&before))])?,
                        _ => one_of(name, fired, &s, &[("unchanged", &as_m(&before)), ("lower", &as_m(&lower))])?,
                    }
                    *smodel = s.into_iter().map(|e| e.0).collect();
                }
                _ => {
                    let keep = |x: u64| x % u64::from(m) != 0;
                    let stop_at = if let Bomb::Closure(c) = bomb { usize::from(c) } else { usize::MAX };
                    let done: BTreeSet<u64> = before.iter().enumerate().filter(|&(i, &x)| keep(x) || i + 1 >= stop_at).map(|(_, &x)| x).collect();
                    fired = armed(name, bomb, || {
                        set.retain(|k| {
                            tick(&CLOSURE, 4, "a set retain closure");
                            keep(k.n())
                        })
                    })?;
                    let s = snap_set(set);
                    if fired == 0 || fired == 4 {
                        one_of(name, fired, &s, &[("after", &as_m(&done))])?;
                    } else {
                        let full: BTreeSet<u64> = before.iter().copied().filter(|&x| keep(x)).collect();
                        prefix_split(name, fired, &as_m(&before), &as_m(&full), &as_m(&before), &s)?;
                    }
                    *smodel = s.into_iter().map(|e| e.0).collect();
                }
            }
            note(fired);
            Ok(name)
        }
        Op::Set { which, k, bomb } => {
            let name = ["set_insert", "set_remove", "set_replace", "set_take"][usize::from(which % 4)];
            let n = ctx.key(k, model);
            let sbefore = smodel.clone();
            let mut swant = sbefore.clone();
            let had = sbefore.contains(&n);
            match which % 4 {
                0 | 2 => {
                    swant.insert(n);
                }
                _ => {
                    swant.remove(&n);
                }
            }
            let got = Cell::new(None);
            let q = K::make(n, mode);
            let fired = armed(name, bomb, || {
                let r = match which % 4 {
                    0 => set.insert(K::make(n, mode)),
                    1 => set.remove(&q),
                    2 => set.replace(K::make(n, mode)).is_some(),
                    _ => set.take(&q).is_some(),
                };
                got.set(Some(r));
            })?;
            drop(q);
            note(fired);
            let s: Vec<(u64, u64)> = set.iter().map(|k| (k.n(), 0)).collect();
            let as_m = |s: &BTreeSet<u64>| s.iter().map(|&k| (k, 0)).collect::<Model>();
            let (mb, mw) = (as_m(&sbefore), as_m(&swant));
            match fired {
                0 => {
                    one_of(name, 0, &s, &[("after", &mw)])?;
                    let want_r = if which % 4 == 0 { !had } else { had };
                    ensure(got.get() == Some(want_r), &format!("upanic__mismatch__{name}"), || format!("returned {:?}, std {want_r}", got.get()))?;
                }
                1 => one_of(name, 1, &s, &[("unchanged", &mb)])?,
                _ => one_of(name, fired, &s, &[("unchanged", &mb), ("after", &mw)])?,
            }
            *smodel = s.into_iter().map(|e| e.0).collect();
            Ok(name)
        }
    }
}
