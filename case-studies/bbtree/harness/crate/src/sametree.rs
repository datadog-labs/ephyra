// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The same tree by other calls. bbtree documents three ways of building a
//! tree that must give the very nodes another way gives:
//!
//! - `insert_many` (maps and sets): "Inserts make the same tree, node by
//!   node, keys and prefix lengths included, as individual inserts"
//!   (`design/12-batch-api.md`), results "exactly as separate calls to
//!   `insert` do" (`unsync/map/batch.rs`).
//! - `extend`, which batches sixteen entries at a time through `insert_many`
//!   once the tree is past the size cutoff (always, under `--cfg fuzzing`),
//!   and inserts the last, partial batch one by one: "Inserts each entry in
//!   turn, as `insert` does".
//! - `Clone`: "node by node, so that the copy has the same shape"
//!   (`tree/clone.rs`).
//!
//! bbtree's own `check_same_nodes` (`--cfg fuzzing`: "for batch equivalence
//! tests and fuzzing") compares length, shape, keys, head keys and prefix
//! lengths; values and tags (keys `Ord` calls equal) compare by `Debug` of
//! the whole contents. Upstream fuzzes this only with 8-entry batches of
//! small trees; here batches of 0 to 128 entries (past the executor's 64
//! pinned lanes), keys near the tree's own (several splits in one leaf in
//! one batch), repeated keys, keys that leave the shared prefix (lanes end
//! early there), over the umodel / typemx / ukey key types, layouts and
//! configs, on trees changed between batches by removals, pops, retain,
//! split_off and clones.
//!
//! Also `get_many` / `contains_many` of 2 to 128 keys against std (the
//! other oracles look up 4 at a time).
//!
//! Keys whose `Ord` panics at a chosen call (`Op::Arm`; `BombKey`,
//! `BombStr`) run the same ops: D48 ("A panic in user code is not
//! reconciled ... a panic need not leave a valid map behind"; "Still sound:
//! no undefined behavior, whatever user code does") leaves only soundness
//! to check after one: the ops go on unchecked, no value is dropped twice
//! and no page freed twice (`sametree__double_drop|double_free__panic`;
//! ASan on odd seeds).
//!
//! Signatures: `sametree__nodes__<op>` (shape differs), `sametree__contents__<op>`
//! (entries or tags differ), `sametree__mismatch__<op>` (a result differs from
//! the individual calls' or std's), `sametree__invariant__<op>`,
//! `sametree__leak__<values|pages>`.

use std::any::TypeId;
use std::cell::Cell;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;
use std::panic::{AssertUnwindSafe, catch_unwind};

use bbtree::InlineKeyStrategy;
use bbtree::unsync::{BBTreeMap, BBTreeSet};
use ephyra::{BoxGen, GenExt, classify, gens};
use serde::{Deserialize, Serialize};

use crate::common::{Fail, MapSeed, ensure, fail, raw_key};
use crate::umodel::{self, Pages, UKey, UVal, eqd};

thread_local! {
    /// `Ord` calls of bomb keys left before one panics; 0 is off.
    static BOMB: Cell<u64> = const { Cell::new(0) };
    /// Whether the bomb went off in this op.
    static FIRED: Cell<bool> = const { Cell::new(false) };
}

fn tick() {
    BOMB.with(|b| match b.get() {
        0 => {}
        1 => {
            b.set(0);
            FIRED.with(|f| f.set(true));
            panic!("sametree: a key's Ord panics (the bomb)");
        }
        n => b.set(n - 1),
    })
}

/// A u64 key whose `Ord` can panic. Under `Auto` (no inline keys for a user
/// type) every comparison calls it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BombKey(u64);
impl PartialOrd for BombKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for BombKey {
    fn cmp(&self, other: &Self) -> Ordering {
        tick();
        self.0.cmp(&other.0)
    }
}
impl UKey for BombKey {
    fn of(raw: u128, mode: u8) -> Self {
        BombKey(<u64 as UKey>::of(raw, mode))
    }
}

/// A byte-string key whose `Ord` can panic; its `InlineKey` is the bytes'
/// (only `Ord` panics: D48 puts hash tags and windows read inside a
/// structural change behind abort-on-unwind guards), so under
/// `WithInlineKey` `Ord` runs on inline-key ties.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BombStr(Vec<u8>);
impl PartialOrd for BombStr {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for BombStr {
    fn cmp(&self, other: &Self) -> Ordering {
        tick();
        self.0.cmp(&other.0)
    }
}
impl bbtree::InlineKey for BombStr {
    const HAS_HASH_TAG: bool = true;
    const HAS_CMP_FROM: bool = true;
    fn inline_key_at(&self, offset: usize) -> bbtree::InlineWord {
        self.0.inline_key_at(offset)
    }
    fn hash_tag(&self) -> u8 {
        self.0.hash_tag()
    }
    fn cmp_from(&self, other: &Self, equal_len: usize) -> Ordering {
        self.0.cmp_from(&other.0, equal_len)
    }
    fn cmp_prefix(&self, prefix: &[u8]) -> Ordering {
        self.0.cmp_prefix(prefix)
    }
}
impl UKey for BombStr {
    fn of(raw: u128, mode: u8) -> Self {
        BombStr(<Vec<u8> as UKey>::of(raw, mode))
    }
}

type Map<K, V, S> = BBTreeMap<K, V, S, Pages>;
type Set<K, S> = BBTreeSet<K, S, Pages>;

/// Batch sizes: the executor pins up to 64 lanes, more sit in `Option`s;
/// N < 2 goes alone through `find_in_batch`.
const NS: [usize; 13] = [0, 1, 2, 3, 5, 8, 9, 16, 17, 33, 64, 65, 128];
/// Lookup batch sizes (`get_many`, `contains_many`).
const LS: [usize; 5] = [2, 9, 33, 65, 128];

/// A key: a raw number, or near one already used in the case (so a batch
/// lands many keys in one leaf).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Pick {
    Raw(u128),
    Near(u16, i8),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Op {
    /// `insert_many` of `NS[n]` entries, keys from `keys` cycled (each
    /// cycle shifted by `shift`), on the map or the set.
    Batch { set: bool, n: u8, keys: Vec<Pick>, shift: u8, v: u64 },
    /// `extend` with `len` keys from `base` by `stride`, with `scatter`
    /// keys put in at their positions; `hint`: an exact size hint, else
    /// none.
    Extend { set: bool, base: Pick, stride: i16, len: u16, scatter: Vec<(u16, Pick)>, hint: bool, v: u64 },
    /// A clone, compared node by node; it replaces the original if `keep`.
    Clone { set: bool, keep: bool },
    Insert { set: bool, k: Pick, v: u64 },
    /// Removes `run` keys from `k` on, stepping raw numbers by `step`.
    Remove { set: bool, k: Pick, run: u16, step: u8 },
    Pop { set: bool, last: bool, n: u16 },
    Retain { set: bool, m: u8, r: u8 },
    /// `split_off` at `k`; keeps the upper part if `upper`.
    SplitOff { set: bool, k: Pick, upper: bool },
    Clear { set: bool },
    /// `get_many` / `contains_many` of `LS[n]` keys, as `Batch` makes them,
    /// against std.
    Lookup { set: bool, n: u8, keys: Vec<Pick>, shift: u8 },
    /// The `after + 1`-th `Ord` call of a bomb key (`BombKey`, `BombStr`)
    /// in the next op panics; a no-op for other keys.
    Arm { after: u16 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Case {
    pub mode: u8,
    pub seed: MapSeed,
    pub ops: Vec<Op>,
}

fn pick() -> BoxGen<Pick> {
    gens::frequency(vec![
        (2, raw_key().map(Pick::Raw).boxed()),
        (3, (gens::int(0u16..=u16::MAX), gens::int(i8::MIN..=i8::MAX)).map(|(i, d)| Pick::Near(i, d)).boxed()),
    ])
    .boxed()
}

fn op() -> BoxGen<Op> {
    let set = gens::frequency(vec![(3, gens::constant(false).boxed()), (1, gens::constant(true).boxed())]);
    let v = gens::int(0u64..=u64::MAX);
    let n = gens::frequency(vec![(3, gens::int(0u8..=11).boxed()), (2, gens::int(0u8..=(NS.len() as u8 - 1)).boxed())]);
    let keys = gens::vec(pick()).len(1..=24);
    let small = gens::int(0u8..=255);
    gens::frequency(vec![
        (
            10,
            (set.clone(), n, keys.clone(), small.clone(), v.clone())
                .map(|(set, n, keys, shift, v)| Op::Batch { set, n, keys, shift, v })
                .boxed(),
        ),
        (
            4,
            (
                set.clone(),
                pick(),
                gens::frequency(vec![(3, gens::int(-3i16..=3).boxed()), (1, gens::int(i16::MIN..=i16::MAX).boxed())]),
                gens::frequency(vec![(3, gens::int(0u16..=40).boxed()), (2, gens::int(0u16..=600).boxed()), (1, gens::int(0u16..=5_000).boxed())]),
                gens::vec((gens::int(0u16..=u16::MAX), pick())).len(0..=8),
                gens::bool(),
                v.clone(),
            )
                .map(|(set, base, stride, len, scatter, hint, v)| Op::Extend { set, base, stride, len, scatter, hint, v })
                .boxed(),
        ),
        (2, (set.clone(), gens::bool()).map(|(set, keep)| Op::Clone { set, keep }).boxed()),
        (3, (set.clone(), pick(), v).map(|(set, k, v)| Op::Insert { set, k, v }).boxed()),
        (
            3,
            (set.clone(), pick(), gens::frequency(vec![(3, gens::int(1u16..=8).boxed()), (1, gens::int(1u16..=3_000).boxed())]), gens::int(1u8..=4))
                .map(|(set, k, run, step)| Op::Remove { set, k, run, step })
                .boxed(),
        ),
        (1, (set.clone(), gens::bool(), gens::int(0u16..=2_000)).map(|(set, last, n)| Op::Pop { set, last, n }).boxed()),
        (1, (set.clone(), gens::int(1u8..=7), small).map(|(set, m, r)| Op::Retain { set, m, r: r % m }).boxed()),
        (1, (set.clone(), pick(), gens::bool()).map(|(set, k, upper)| Op::SplitOff { set, k, upper }).boxed()),
        (1, set.clone().map(|set| Op::Clear { set }).boxed()),
        (
            2,
            gens::frequency(vec![(2, gens::int(0u16..=40).boxed()), (1, gens::int(0u16..=4_000).boxed())]).map(|after| Op::Arm { after }).boxed(),
        ),
        (
            3,
            (set, gens::int(0u8..=(LS.len() as u8 - 1)), keys, gens::int(0u8..=255))
                .map(|(set, n, keys, shift)| Op::Lookup { set, n, keys, shift })
                .boxed(),
        ),
    ])
    .boxed()
}

pub fn case() -> BoxGen<Case> {
    (gens::int(0u8..=3), MapSeed::generator(), gens::vec(op()).len(1..=30))
        .iso(|(mode, seed, ops)| Case { mode, seed, ops }, |c: &Case| (c.mode, c.seed.clone(), c.ops.clone()))
        .boxed()
}

fn panic_msg(p: Box<dyn std::any::Any + Send>) -> String {
    p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_else(|| "?".into())
}

/// Runs bbtree's own check (it panics), as a failure of `sig`.
fn check(sig: &str, at: &dyn Fn(String) -> String, f: impl FnOnce()) -> Result<(), Fail> {
    catch_unwind(AssertUnwindSafe(f)).map_err(|p| fail(sig, at(panic_msg(p))))
}

#[allow(unused_variables)]
fn same_nodes_map<K: UKey, V: UVal, S: InlineKeyStrategy<K>>(a: &Map<K, V, S>, b: &Map<K, V, S>, area: &str, at: &dyn Fn(String) -> String) -> Result<(), Fail> {
    #[cfg(fuzzing)]
    {
        check(&format!("sametree__nodes__{area}"), at, || a.check_same_nodes(b))?;
        check(&format!("sametree__invariant__{area}"), at, || {
            a.check_invariants();
            a.check_prefix_lens();
        })?;
    }
    let pos = a.iter().zip(b.iter()).position(|((ka, va), (kb, vb))| !(eqd(ka, kb) && va.n() == vb.n()));
    ensure(a.len() == b.len() && pos.is_none(), &format!("sametree__contents__{area}"), || {
        let p = pos.unwrap_or(0);
        at(format!("len {} vs {}; entry {p}: {:?} vs {:?}", a.len(), b.len(), a.iter().nth(p), b.iter().nth(p)))
    })
}

#[allow(unused_variables)]
fn same_nodes_set<K: UKey, S: InlineKeyStrategy<K>>(a: &Set<K, S>, b: &Set<K, S>, area: &str, at: &dyn Fn(String) -> String) -> Result<(), Fail> {
    #[cfg(fuzzing)]
    {
        check(&format!("sametree__nodes__{area}"), at, || a.check_same_nodes(b))?;
        check(&format!("sametree__invariant__{area}"), at, || a.check_invariants())?;
    }
    let pos = a.iter().zip(b.iter()).position(|(x, y)| !eqd(x, y));
    ensure(a.len() == b.len() && pos.is_none(), &format!("sametree__contents__{area}"), || {
        let p = pos.unwrap_or(0);
        at(format!("len {} vs {}; element {p}: {:?} vs {:?}", a.len(), b.len(), a.iter().nth(p), b.iter().nth(p)))
    })
}

fn vs_model_map<K: UKey, V: UVal, S: InlineKeyStrategy<K>>(map: &Map<K, V, S>, model: &BTreeMap<K, u64>, area: &str, at: &dyn Fn(String) -> String) -> Result<(), Fail> {
    let pos = map.iter().zip(model.iter()).position(|((ka, va), (kb, vb))| !(eqd(ka, kb) && va.n() == *vb));
    ensure(map.len() == model.len() && pos.is_none(), &format!("sametree__mismatch__{area}"), || {
        let p = pos.unwrap_or(0);
        at(format!("vs std: len {} vs {}; entry {p}: {:?} vs {:?}", map.len(), model.len(), map.iter().nth(p), model.iter().nth(p)))
    })
}

fn vs_model_set<K: UKey, S: InlineKeyStrategy<K>>(set: &Set<K, S>, model: &BTreeSet<K>, area: &str, at: &dyn Fn(String) -> String) -> Result<(), Fail> {
    let pos = set.iter().zip(model.iter()).position(|(x, y)| !eqd(x, y));
    ensure(set.len() == model.len() && pos.is_none(), &format!("sametree__mismatch__{area}"), || {
        let p = pos.unwrap_or(0);
        at(format!("vs std: len {} vs {}; element {p}: {:?} vs {:?}", set.len(), model.len(), set.iter().nth(p), model.iter().nth(p)))
    })
}

/// `insert_many` on the map against a clone given the same entries one
/// `insert` at a time, and std.
fn batch_map<const N: usize, K: UKey, V: UVal, S: InlineKeyStrategy<K>>(
    map: &mut Map<K, V, S>,
    model: &mut BTreeMap<K, u64>,
    entries: &[(K, u64)],
    at: &dyn Fn(String) -> String,
) -> Result<(), Fail> {
    let mut individual = map.clone();
    same_nodes_map(&individual, map, "clone", at)?;
    let arr: [(K, V); N] = std::array::from_fn(|j| (entries[j].0.clone(), V::new(entries[j].1)));
    let x: [Option<u64>; N] = map.insert_many(arr).map(|o| o.map(|v| v.n()));
    let y: [Option<u64>; N] = std::array::from_fn(|j| individual.insert(entries[j].0.clone(), V::new(entries[j].1)).map(|v| v.n()));
    let z: [Option<u64>; N] = std::array::from_fn(|j| model.insert(entries[j].0.clone(), entries[j].1));
    ensure(x == y && y == z, "sametree__mismatch__insert_many", || at(format!("batch {x:?}, one by one {y:?}, std {z:?}")))?;
    same_nodes_map(map, &individual, "insert_many", at)?;
    vs_model_map(map, model, "insert_many", at)
}

fn batch_set<const N: usize, K: UKey, S: InlineKeyStrategy<K>>(
    set: &mut Set<K, S>,
    model: &mut BTreeSet<K>,
    entries: &[(K, u64)],
    at: &dyn Fn(String) -> String,
) -> Result<(), Fail> {
    let mut individual = set.clone();
    same_nodes_set(&individual, set, "set_clone", at)?;
    let arr: [K; N] = std::array::from_fn(|j| entries[j].0.clone());
    let x: [bool; N] = set.insert_many(arr);
    let y: [bool; N] = std::array::from_fn(|j| individual.insert(entries[j].0.clone()));
    let z: [bool; N] = std::array::from_fn(|j| model.insert(entries[j].0.clone()));
    ensure(x == y && y == z, "sametree__mismatch__set_insert_many", || at(format!("batch {x:?}, one by one {y:?}, std {z:?}")))?;
    same_nodes_set(set, &individual, "set_insert_many", at)?;
    vs_model_set(set, model, "set_insert_many", at)
}

fn lookup_map<const N: usize, K: UKey, V: UVal, S: InlineKeyStrategy<K>>(map: &Map<K, V, S>, model: &BTreeMap<K, u64>, keys: &[K], at: &dyn Fn(String) -> String) -> Result<(), Fail> {
    let qs: [&K; N] = std::array::from_fn(|j| &keys[j]);
    let x: [Option<u64>; N] = map.get_many(qs).map(|o| o.map(|v| v.n()));
    let y: [Option<u64>; N] = qs.map(|k| model.get(k).copied());
    ensure(x == y, "sametree__mismatch__get_many", || at(format!("{x:?} vs std {y:?}")))
}

fn lookup_set<const N: usize, K: UKey, S: InlineKeyStrategy<K>>(set: &Set<K, S>, model: &BTreeSet<K>, keys: &[K], at: &dyn Fn(String) -> String) -> Result<(), Fail> {
    let qs: [&K; N] = std::array::from_fn(|j| &keys[j]);
    let x: [bool; N] = set.contains_many(qs);
    let y: [bool; N] = qs.map(|k| model.contains(k));
    ensure(x == y, "sametree__mismatch__contains_many", || at(format!("{x:?} vs std {y:?}")))
}

macro_rules! by_n {
    ($n:expr, $f:ident, $($arg:expr),*) => {
        match $n {
            0 => $f::<0, _, _, _>($($arg),*),
            1 => $f::<1, _, _, _>($($arg),*),
            2 => $f::<2, _, _, _>($($arg),*),
            3 => $f::<3, _, _, _>($($arg),*),
            5 => $f::<5, _, _, _>($($arg),*),
            8 => $f::<8, _, _, _>($($arg),*),
            9 => $f::<9, _, _, _>($($arg),*),
            16 => $f::<16, _, _, _>($($arg),*),
            17 => $f::<17, _, _, _>($($arg),*),
            33 => $f::<33, _, _, _>($($arg),*),
            64 => $f::<64, _, _, _>($($arg),*),
            65 => $f::<65, _, _, _>($($arg),*),
            _ => $f::<128, _, _, _>($($arg),*),
        }
    };
}
macro_rules! by_n_set {
    ($n:expr, $f:ident, $($arg:expr),*) => {
        match $n {
            0 => $f::<0, _, _>($($arg),*),
            1 => $f::<1, _, _>($($arg),*),
            2 => $f::<2, _, _>($($arg),*),
            3 => $f::<3, _, _>($($arg),*),
            5 => $f::<5, _, _>($($arg),*),
            8 => $f::<8, _, _>($($arg),*),
            9 => $f::<9, _, _>($($arg),*),
            16 => $f::<16, _, _>($($arg),*),
            17 => $f::<17, _, _>($($arg),*),
            33 => $f::<33, _, _>($($arg),*),
            64 => $f::<64, _, _>($($arg),*),
            65 => $f::<65, _, _>($($arg),*),
            _ => $f::<128, _, _>($($arg),*),
        }
    };
}
macro_rules! by_l {
    ($n:expr, $f:ident, $($arg:expr),*) => {
        match $n {
            2 => $f::<2, _, _, _>($($arg),*),
            9 => $f::<9, _, _, _>($($arg),*),
            33 => $f::<33, _, _, _>($($arg),*),
            65 => $f::<65, _, _, _>($($arg),*),
            _ => $f::<128, _, _, _>($($arg),*),
        }
    };
}
macro_rules! by_l_set {
    ($n:expr, $f:ident, $($arg:expr),*) => {
        match $n {
            2 => $f::<2, _, _>($($arg),*),
            9 => $f::<9, _, _>($($arg),*),
            33 => $f::<33, _, _>($($arg),*),
            65 => $f::<65, _, _>($($arg),*),
            _ => $f::<128, _, _>($($arg),*),
        }
    };
}

/// Inserts `es` through `insert_many` in batches of `C`, the rest one by one.
fn bare_many<const C: usize, K: UKey, V: UVal, S: InlineKeyStrategy<K>>(map: &mut Map<K, V, S>, set: &mut Set<K, S>, on_set: bool, es: &mut Vec<(K, u64)>) {
    while es.len() >= C {
        let mut it = es.drain(..C);
        if on_set {
            set.insert_many(std::array::from_fn::<_, C, _>(|_| it.next().expect("C entries").0));
        } else {
            map.insert_many(std::array::from_fn::<_, C, _>(|_| {
                let (k, v) = it.next().expect("C entries");
                (k, V::new(v))
            }));
        }
    }
}

/// An op with no model and no check: after a key's `Ord` panicked (D48).
fn bare_op<K: UKey, V: UVal, S: InlineKeyStrategy<K>>(map: &mut Map<K, V, S>, set: &mut Set<K, S>, op: &Op, key: &dyn Fn(u128) -> K, resolve: &dyn Fn(Pick) -> u128) {
    let made = |keys: &[Pick], n: usize, shift: u8, v: u64| -> Vec<(K, u64)> {
        let picked: Vec<u128> = keys.iter().map(|&p| resolve(p)).collect();
        (0..n).map(|j| (key(picked[j % picked.len()].wrapping_add((j / picked.len()) as u128 * u128::from(shift))), v.wrapping_add(j as u64))).collect()
    };
    match op {
        Op::Arm { .. } => {}
        Op::Batch { set: on_set, n, keys, shift, v } => {
            let mut es = made(keys, NS[*n as usize % NS.len()], *shift, *v);
            bare_many::<65, _, _, _>(map, set, *on_set, &mut es);
            bare_many::<16, _, _, _>(map, set, *on_set, &mut es);
            bare_many::<3, _, _, _>(map, set, *on_set, &mut es);
            bare_many::<1, _, _, _>(map, set, *on_set, &mut es);
        }
        Op::Lookup { set: on_set, n, keys, shift } => {
            let es = made(keys, LS[*n as usize % LS.len()], *shift, 0);
            for c in es.chunks_exact(9) {
                let qs: [&K; 9] = std::array::from_fn(|j| &c[j].0);
                if *on_set {
                    set.contains_many(qs);
                } else {
                    map.get_many(qs);
                }
            }
        }
        Op::Extend { set: on_set, base, stride, len, scatter, hint, v } => {
            let b = resolve(*base);
            let mut rs: Vec<u128> = (0..i128::from(*len)).map(|j| b.wrapping_add_signed(j * i128::from(*stride))).collect();
            for &(pos, p) in scatter {
                rs.insert(pos as usize % (rs.len() + 1), resolve(p));
            }
            if *on_set {
                let ks = rs.iter().map(|&r| key(r));
                if *hint { set.extend(ks.collect::<Vec<_>>()) } else { set.extend(ks) }
            } else {
                let es = rs.iter().enumerate().map(|(j, &r)| (key(r), V::new(v.wrapping_add(j as u64))));
                if *hint { map.extend(es.collect::<Vec<_>>()) } else { map.extend(es) }
            }
        }
        Op::Clone { set: on_set, keep } => {
            if *on_set {
                let c = set.clone();
                if *keep {
                    *set = c;
                }
            } else {
                let c = map.clone();
                if *keep {
                    *map = c;
                }
            }
        }
        Op::Insert { set: on_set, k, v } => {
            let k = key(resolve(*k));
            if *on_set {
                set.insert(k);
            } else {
                map.insert(k, V::new(*v));
            }
        }
        Op::Remove { set: on_set, k, run, step } => {
            let r0 = resolve(*k);
            for j in 0..u128::from(*run) {
                let k = key(r0.wrapping_add(j * u128::from(*step)));
                if *on_set {
                    set.remove(&k);
                } else {
                    map.remove(&k);
                }
            }
        }
        Op::Pop { set: on_set, last, n } => {
            for _ in 0..*n {
                match (*on_set, *last) {
                    (true, true) => drop(set.pop_last()),
                    (true, false) => drop(set.pop_first()),
                    (false, true) => drop(map.pop_last()),
                    (false, false) => drop(map.pop_first()),
                }
            }
        }
        Op::Retain { set: on_set, m, r } => {
            let (m, r) = (u64::from(*m), u64::from(*r));
            if *on_set {
                let mut c = 0u64;
                set.retain(|_| {
                    c += 1;
                    c % m != r
                });
            } else {
                map.retain(|_, v| v.n() % m != r);
            }
        }
        Op::SplitOff { set: on_set, k, upper } => {
            let k = key(resolve(*k));
            if *on_set {
                let hi = set.split_off(&k);
                if *upper {
                    *set = hi;
                }
            } else {
                let hi = map.split_off(&k);
                if *upper {
                    *map = hi;
                }
            }
        }
        Op::Clear { set: on_set } => {
            if *on_set {
                set.clear();
            } else {
                map.clear();
            }
        }
    }
}

pub fn run<K: UKey, V: UVal, S: InlineKeyStrategy<K> + Default>(case: &Case) -> Result<(), Fail> {
    umodel::reset::<K>();
    BOMB.with(|b| b.set(0));
    let tainted = run_inner::<K, V, S>(case);
    BOMB.with(|b| b.set(0));
    let tainted = tainted?;
    let (live, pages) = (umodel::live(), umodel::pages());
    if tainted {
        // D48: after a panic in `Ord` the map "does whatever is simplest",
        // but stays sound: no value dropped twice, no page freed twice.
        ensure(live >= 0, "sametree__double_drop__panic", || format!("{live} values alive after every collection dropped"))?;
        return ensure(pages >= 0, "sametree__double_free__panic", || format!("{pages} pages out after every collection dropped"));
    }
    ensure(live == 0, "sametree__leak__values", || format!("{live} values alive after every collection dropped"))?;
    ensure(pages == 0, "sametree__leak__pages", || format!("{pages} pages out after every collection dropped"))
}

/// Whether a key's `Ord` panicked (the bomb), after which only soundness
/// counts.
fn run_inner<K: UKey, V: UVal, S: InlineKeyStrategy<K> + Default>(case: &Case) -> Result<bool, Fail> {
    let mode = case.mode;
    let key = |r: u128| K::of(r, mode);
    let mut map: Map<K, V, S> = BBTreeMap::with_strategy_in(S::default(), Pages);
    let mut set: Set<K, S> = BBTreeSet::with_strategy_in(S::default(), Pages);
    let mut model: BTreeMap<K, u64> = BTreeMap::new();
    let mut smodel: BTreeSet<K> = BTreeSet::new();
    // Every raw number used so far, for `Pick::Near`.
    let mut raws: Vec<u128> = Vec::new();
    for raw in case.seed.raw_keys() {
        let (k, v) = (key(raw), case.seed.value_of(raw));
        map.insert(k.clone(), V::new(v));
        model.insert(k.clone(), v);
        if raw % 3 != 0 {
            set.insert(k.clone());
            smodel.insert(k);
        }
        raws.push(raw);
    }
    let (mut big_batch, mut nonempty) = (false, false);
    let (mut armed, mut tainted, mut ord_panicked) = (0u64, false, false);
    let bomb_key = [TypeId::of::<BombKey>(), TypeId::of::<BombStr>()].contains(&TypeId::of::<K>());
    for (i, op) in case.ops.iter().enumerate() {
        umodel::set_tag(i as u32 + 1);
        if let Op::Arm { after } = op {
            if bomb_key {
                armed = u64::from(*after) + 1;
            }
            continue;
        }
        let at = |why: String| format!("op {i} {op:?}: {why}");
        let resolve = |p: Pick, raws: &[u128]| match p {
            Pick::Raw(r) => r,
            Pick::Near(j, d) if !raws.is_empty() => raws[j as usize % raws.len()].wrapping_add_signed(i128::from(d)),
            Pick::Near(_, d) => d as u128,
        };
        if armed > 0 || tainted {
            // The op alone, no model or check (they call `Ord` too, and
            // std's map is no more reconciled after a panic than bbtree's).
            // The models are stale from here on.
            BOMB.with(|b| b.set(std::mem::take(&mut armed)));
            FIRED.with(|f| f.set(false));
            let r = catch_unwind(AssertUnwindSafe(|| bare_op(&mut map, &mut set, op, &key, &|p| resolve(p, &raws))));
            BOMB.with(|b| b.set(0));
            let fired = FIRED.with(|f| f.get());
            if let Err(p) = r
                && !tainted
                && !fired
            {
                std::panic::resume_unwind(p);
            }
            (tainted, ord_panicked) = (true, ord_panicked | fired);
            ensure(umodel::live() >= 0, "sametree__double_drop__panic", || at(format!("{} values alive", umodel::live())))?;
            continue;
        }
        let r = catch_unwind(AssertUnwindSafe(|| -> Result<(), Fail> {
        match op {
            Op::Arm { .. } => {}
            Op::Batch { set: on_set, n, keys, shift, v } => {
                let n = NS[*n as usize % NS.len()];
                let picked: Vec<u128> = keys.iter().map(|&p| resolve(p, &raws)).collect();
                let entries: Vec<(K, u64)> = (0..n)
                    .map(|j| {
                        let cycle = (j / picked.len()) as u128;
                        let r = picked[j % picked.len()].wrapping_add(cycle * u128::from(*shift));
                        (key(r), v.wrapping_add(j as u64))
                    })
                    .collect();
                raws.extend(picked);
                big_batch |= n > 64;
                nonempty |= if *on_set { !set.is_empty() } else { !map.is_empty() };
                if *on_set {
                    by_n_set!(n, batch_set, &mut set, &mut smodel, &entries, &at)?;
                } else {
                    by_n!(n, batch_map, &mut map, &mut model, &entries, &at)?;
                }
            }
            Op::Lookup { set: on_set, n, keys, shift } => {
                let n = LS[*n as usize % LS.len()];
                let picked: Vec<u128> = keys.iter().map(|&p| resolve(p, &raws)).collect();
                let ks: Vec<K> = (0..n)
                    .map(|j| key(picked[j % picked.len()].wrapping_add((j / picked.len()) as u128 * u128::from(*shift))))
                    .collect();
                if *on_set {
                    by_l_set!(n, lookup_set, &set, &smodel, &ks, &at)?;
                } else {
                    by_l!(n, lookup_map, &map, &model, &ks, &at)?;
                }
            }
            Op::Extend { set: on_set, base, stride, len, scatter, hint, v } => {
                let b = resolve(*base, &raws);
                let mut rs: Vec<u128> = (0..i128::from(*len)).map(|j| b.wrapping_add_signed(j * i128::from(*stride))).collect();
                for &(pos, p) in scatter {
                    let r = resolve(p, &raws);
                    rs.insert(pos as usize % (rs.len() + 1), r);
                }
                let entries: Vec<(K, u64)> = rs.iter().enumerate().map(|(j, &r)| (key(r), v.wrapping_add(j as u64))).collect();
                raws.push(b);
                if *on_set {
                    let mut individual = set.clone();
                    same_nodes_set(&individual, &set, "set_clone", &at)?;
                    let ks = entries.iter().map(|(k, _)| k.clone());
                    if *hint {
                        set.extend(ks.collect::<Vec<_>>());
                    } else {
                        let mut it = ks;
                        set.extend(std::iter::from_fn(move || it.next()));
                    }
                    for (k, _) in &entries {
                        individual.insert(k.clone());
                        smodel.insert(k.clone());
                    }
                    same_nodes_set(&set, &individual, "set_extend", &at)?;
                    vs_model_set(&set, &smodel, "set_extend", &at)?;
                } else {
                    let mut individual = map.clone();
                    same_nodes_map(&individual, &map, "clone", &at)?;
                    let es = entries.iter().map(|(k, v)| (k.clone(), V::new(*v)));
                    if *hint {
                        map.extend(es.collect::<Vec<_>>());
                    } else {
                        let mut it = es;
                        map.extend(std::iter::from_fn(move || it.next()));
                    }
                    for (k, v) in &entries {
                        individual.insert(k.clone(), V::new(*v));
                        model.insert(k.clone(), *v);
                    }
                    same_nodes_map(&map, &individual, "extend", &at)?;
                    vs_model_map(&map, &model, "extend", &at)?;
                }
            }
            Op::Clone { set: on_set, keep } => {
                if *on_set {
                    let copy = set.clone();
                    same_nodes_set(&copy, &set, "set_clone", &at)?;
                    if *keep {
                        set = copy;
                    }
                } else {
                    let copy = map.clone();
                    same_nodes_map(&copy, &map, "clone", &at)?;
                    if *keep {
                        map = copy;
                    }
                }
            }
            Op::Insert { set: on_set, k, v } => {
                let r = resolve(*k, &raws);
                raws.push(r);
                if *on_set {
                    let (x, y) = (set.insert(key(r)), smodel.insert(key(r)));
                    ensure(x == y, "sametree__mismatch__set_insert", || at(format!("{x} vs {y}")))?;
                } else {
                    let x = map.insert(key(r), V::new(*v)).map(|v| v.n());
                    let y = model.insert(key(r), *v);
                    ensure(x == y, "sametree__mismatch__insert", || at(format!("{x:?} vs {y:?}")))?;
                }
            }
            Op::Remove { set: on_set, k, run, step } => {
                let r0 = resolve(*k, &raws);
                for j in 0..u128::from(*run) {
                    let k = key(r0.wrapping_add(j * u128::from(*step)));
                    if *on_set {
                        let (x, y) = (set.remove(&k), smodel.remove(&k));
                        ensure(x == y, "sametree__mismatch__set_remove", || at(format!("{x} vs {y}")))?;
                    } else {
                        let x = map.remove(&k).map(|v| v.n());
                        let y = model.remove(&k);
                        ensure(x == y, "sametree__mismatch__remove", || at(format!("{x:?} vs {y:?}")))?;
                    }
                }
            }
            Op::Pop { set: on_set, last, n } => {
                for _ in 0..*n {
                    if *on_set {
                        let (x, y) = if *last { (set.pop_last(), smodel.pop_last()) } else { (set.pop_first(), smodel.pop_first()) };
                        ensure(eqd(&x, &y), "sametree__mismatch__set_pop", || at(format!("{x:?} vs {y:?}")))?;
                    } else {
                        let (x, y) = if *last { (map.pop_last(), model.pop_last()) } else { (map.pop_first(), model.pop_first()) };
                        let x = x.map(|(k, v)| (k, v.n()));
                        ensure(eqd(&x, &y), "sametree__mismatch__pop", || at(format!("{x:?} vs {y:?}")))?;
                    }
                }
            }
            Op::Retain { set: on_set, m, r } => {
                let (m, r) = (u64::from(*m), u64::from(*r));
                if *on_set {
                    let mut c = 0u64;
                    set.retain(|_| {
                        c += 1;
                        c % m != r
                    });
                    let mut c = 0u64;
                    smodel.retain(|_| {
                        c += 1;
                        c % m != r
                    });
                } else {
                    map.retain(|_, v| v.n() % m != r);
                    model.retain(|_, v| *v % m != r);
                }
            }
            Op::SplitOff { set: on_set, k, upper } => {
                let k = key(resolve(*k, &raws));
                if *on_set {
                    let (hi, mhi) = (set.split_off(&k), smodel.split_off(&k));
                    if *upper {
                        (set, smodel) = (hi, mhi);
                    }
                } else {
                    let (hi, mhi) = (map.split_off(&k), model.split_off(&k));
                    if *upper {
                        (map, model) = (hi, mhi);
                    }
                }
            }
            Op::Clear { set: on_set } => {
                if *on_set {
                    set.clear();
                    smodel.clear();
                } else {
                    map.clear();
                    model.clear();
                }
            }
        }
        if !matches!(op, Op::Batch { .. } | Op::Extend { .. } | Op::Lookup { .. }) && i % 4 == 3 {
            vs_model_map(&map, &model, "state", &at)?;
            vs_model_set(&set, &smodel, "state", &at)?;
        }
        Ok(())
        }));
        BOMB.with(|b| b.set(0));
        match r {
            Ok(r) => r?,
            Err(p) => std::panic::resume_unwind(p),
        }
    }
    classify(ord_panicked, "ord_panicked");
    if !tainted {
        let at = |why: String| format!("end: {why}");
        vs_model_map(&map, &model, "state", &at)?;
        vs_model_set(&set, &smodel, "state", &at)?;
    }
    classify(big_batch, "big_batch");
    classify(nonempty, "batch_on_nonempty");
    classify(map.len() >= 1000, "map_1k");
    drop((map, set));
    Ok(tainted)
}
