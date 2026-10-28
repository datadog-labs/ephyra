// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! pagemx: the exact models of other oracles (umodel's stateful model,
//! sametree's same nodes, bulkmap's large removal phases, the file-map round
//! trip, the store differential, synclin's threads) over node configs no
//! other property uses. The others' pages are at most 8 KiB, so with their
//! keys a node holds at most a few hundred. `NodeConfig` takes any power of
//! two from its minimum up ("Larger leaves are fewer, but an insert or a
//! remove shifts more of a leaf's bytes"): here leaves of 16 KiB to 256 KiB
//! and internal nodes of 16 KiB to 64 KiB, alone and mixed with the
//! smallest of the other kind, so that small keys fill nodes up to the
//! 1023-key cap (`layout.rs` `MAX_CAPACITY`), searches span many vector
//! words, and splits and merges move hundreds of keys at once.
//!
//! Every failure keeps the oracle it came from in its area:
//! `pagemx__<kind>__<oracle>_<area>`.

use bbtree::{Auto, Configured, InlineKeyStrategy, NodeConfig, WithInlineKey};

use crate::common::Fail;
use crate::{bulk, filemap, sametree, store, synclin, syncthreads, umodel};

/// Leaves of 16 KiB, default internal nodes.
#[derive(Default)]
pub struct L16K;
impl NodeConfig for L16K {
    const LEAF_PAGE_SIZE: usize = 16384;
}

/// Leaves of 64 KiB under the smallest internal nodes: many levels of
/// 12-key internal nodes over 1023-key leaves.
#[derive(Default)]
pub struct L64KTiny;
impl NodeConfig for L64KTiny {
    const LEAF_PAGE_SIZE: usize = 65536;
    const INTERNAL_PAGE_SIZE: usize = 64;
    const PLAIN_INTERNAL_PAGE_SIZE: usize = 64;
}

/// Leaves of 256 KiB, internal nodes of 64 KiB.
#[derive(Default)]
pub struct Huge;
impl NodeConfig for Huge {
    const LEAF_PAGE_SIZE: usize = 262144;
    const INTERNAL_PAGE_SIZE: usize = 65536;
    const PLAIN_INTERNAL_PAGE_SIZE: usize = 65536;
}

/// The smallest leaves under internal nodes of 64 KiB.
#[derive(Default)]
pub struct I64K;
impl NodeConfig for I64K {
    const INTERNAL_PAGE_SIZE: usize = 65536;
    const PLAIN_INTERNAL_PAGE_SIZE: usize = 65536;
}

/// Leaves of 32 KiB, inline-key internal nodes of 16 KiB, plain ones of
/// 2 KiB, heap pages of the minimum 1 KiB.
#[derive(Default)]
pub struct Mixed;
impl NodeConfig for Mixed {
    const LEAF_PAGE_SIZE: usize = 32768;
    const INTERNAL_PAGE_SIZE: usize = 16384;
    const PLAIN_INTERNAL_PAGE_SIZE: usize = 2048;
    const HEAP_PAGE_SIZE: usize = 1024;
}

/// A store with leaves and heap pages of 64 KiB: a quarter of a heap page
/// is its `max_key_len`, and values up to 20 KB stay out of overflow pages.
#[derive(Default)]
pub struct StoreHuge;
impl NodeConfig for StoreHuge {
    const LEAF_PAGE_SIZE: usize = 65536;
    const HEAP_PAGE_SIZE: usize = 65536;
}

pub type A<C> = Configured<Auto, C>;
pub type I<C> = Configured<WithInlineKey, C>;

/// `<oracle>__<kind>__<area>` as `pagemx__<kind>__<oracle>_<area>`.
fn retag(mut f: Fail) -> Fail {
    let parts: Vec<&str> = f.sig.splitn(3, "__").collect();
    if let [oracle, kind, area] = parts[..] {
        f.sig = format!("pagemx__{kind}__{oracle}_{area}");
    } else {
        f.sig = format!("pagemx__{}", f.sig);
    }
    f
}

impl bulk::Key for u32 {
    /// Whole-compare keys: no inline keys under `Auto`, so internal nodes
    /// keep the keys themselves (`PLAIN_INTERNAL_PAGE_SIZE`).
    fn of(n: u32) -> Self {
        n.wrapping_mul(7)
    }
}

pub fn umodel<K: umodel::UKey, V: umodel::UVal, S: InlineKeyStrategy<K> + Default>(case: &umodel::Case) -> Result<(), Fail> {
    umodel::run::<K, V, S>(case).map_err(retag)
}

pub fn umodel_sync<K: umodel::UKey + Send + Sync, V: umodel::UVal + Send + Sync, S: InlineKeyStrategy<K> + Default>(
    case: &umodel::Case,
) -> Result<(), Fail> {
    umodel::run_sync::<K, V, S>(case).map_err(retag)
}

pub fn sametree<K: umodel::UKey, V: umodel::UVal, S: InlineKeyStrategy<K> + Default>(case: &sametree::Case) -> Result<(), Fail> {
    sametree::run::<K, V, S>(case).map_err(retag)
}

pub fn bulk<K: bulk::Key, S: InlineKeyStrategy<K> + Default>(case: &bulk::Case) -> Result<(), Fail> {
    bulk::run::<K, S>(case).map_err(retag)
}

pub fn filemap<K, V, S>(case: &filemap::RoundtripCase) -> Result<(), Fail>
where
    K: crate::common::FromRaw + bbtree::file::PlainBytes + bbtree::file::Storable<InWriter = K>,
    V: crate::common::FromRaw + bbtree::file::PlainBytes + bbtree::file::Storable<InWriter = V> + PartialEq,
    S: InlineKeyStrategy<K> + Default,
{
    filemap::roundtrip::<K, V, S>(case).map_err(retag)
}

pub fn store<C: NodeConfig + Default>(case: &store::Case) -> Result<(), Fail> {
    store::run::<C>(case).map_err(retag)
}

pub fn synclin<K: syncthreads::Key, S: InlineKeyStrategy<K> + Default + Send + Sync, const N: usize, const SHRINK: bool>(
    case: &synclin::Case,
) -> Result<(), Fail>
where
    bbtree::BBTreeMap<K, synclin::W<N>, S>: Sync,
{
    synclin::run::<K, S, N, SHRINK>(case).map_err(retag)
}

// ---------- long keys: shared runs around the 65535-byte node prefix cap ----------

/// Keys of two runs of a fill byte, each followed by a byte of the key's
/// number: keys whose high bytes agree share `a + 1 + b` bytes, so with `a`
/// or `a + 1 + b` near 65535 the fences of a node share more than a node
/// records (`page.rs` `MAX_PREFIX_LEN`, a `u16`; `common_prefix_len`
/// "stopping short is always safe").
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LongCase {
    pub a: u32,
    pub b: u32,
    pub ops: Vec<LongOp>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum LongOp {
    Insert(u16, u64),
    /// `n` keys from `start` stepping by `step`.
    Run { start: u16, step: u8, n: u8 },
    Remove(u16),
    Get(u16),
    /// A double-ended walk over `lo..=hi`.
    Range { lo: u16, hi: u16, walk: Vec<bool> },
    Pop(bool),
    SplitAppend(u16),
}

fn run_len() -> ephyra::BoxGen<u32> {
    use ephyra::{GenExt, gens};
    gens::frequency(vec![
        (1, gens::constant(0u32).boxed()),
        (1, gens::int(0u32..=300).boxed()),
        (4, gens::int(65_520u32..=65_545).boxed()),
        (1, gens::int(65_000u32..=66_100).boxed()),
        (1, gens::constant(131_071u32).boxed()),
    ])
    .boxed()
}

pub fn long_case() -> ephyra::BoxGen<LongCase> {
    use ephyra::{GenExt, gens};
    let k = gens::int(0u16..=u16::MAX);
    let op = gens::frequency(vec![
        (5, (k.clone(), gens::int(0u64..=u64::MAX)).map(|(k, v)| LongOp::Insert(k, v)).boxed()),
        (3, (k.clone(), gens::int(1u8..=255), gens::int(1u8..=120)).map(|(start, step, n)| LongOp::Run { start, step, n }).boxed()),
        (3, k.clone().map(LongOp::Remove).boxed()),
        (3, k.clone().map(LongOp::Get).boxed()),
        (2, (k.clone(), k.clone(), gens::vec(gens::bool()).len(0..=12)).map(|(a, b, walk)| LongOp::Range { lo: a.min(b), hi: a.max(b), walk }).boxed()),
        (1, gens::bool().map(LongOp::Pop).boxed()),
        (1, k.map(LongOp::SplitAppend).boxed()),
    ]);
    (run_len(), run_len(), gens::vec(op).len(1..=40))
        .iso(|(a, b, ops)| LongCase { a, b, ops }, |c: &LongCase| (c.a, c.b, c.ops.clone()))
        .boxed()
}

fn long_key(c: &LongCase, n: u16) -> Vec<u8> {
    let mut k = Vec::with_capacity(c.a as usize + c.b as usize + 2);
    k.resize(c.a as usize, b'p');
    // the fill is not 0: "ab" and "ab\0" pad alike
    k.push((n >> 8) as u8);
    k.resize(k.len() + c.b as usize, b'q');
    k.push(n as u8);
    k
}

fn long_fail(oracle: &str, kind: &str, area: &str, msg: String) -> Fail {
    crate::common::fail(&format!("pagemx__{kind}__{oracle}_{area}"), msg)
}

fn short(k: &[u8]) -> String {
    let fill = k.iter().take_while(|&&b| b == b'p').count();
    format!("p*{fill} {:?} (len {})", &k[fill..(fill + 1).min(k.len())], k.len())
}

/// The unsync map against std, `check_invariants` / `check_prefix_lens`
/// after every op.
pub fn long_unsync<S: InlineKeyStrategy<Vec<u8>> + Default>(c: &LongCase) -> Result<(), Fail> {
    use std::collections::BTreeMap;
    let mut map = bbtree::unsync::BBTreeMap::<Vec<u8>, u64, S>::with_strategy(S::default());
    let mut model = BTreeMap::<Vec<u8>, u64>::new();
    let o = "long";
    for (i, op) in c.ops.iter().enumerate() {
        let at = |what: &str| format!("op {i} {what}");
        match op {
            LongOp::Insert(n, v) => {
                let k = long_key(c, *n);
                let (ours, want) = (map.insert(k.clone(), *v), model.insert(k, *v));
                if ours != want {
                    return Err(long_fail(o, "mismatch", "insert", format!("{}: ours {ours:?}, std {want:?}", at("insert"))));
                }
            }
            LongOp::Run { start, step, n } => {
                for j in 0..*n as u16 {
                    let k = long_key(c, start.wrapping_add(j.wrapping_mul(*step as u16)));
                    let v = j as u64;
                    if map.insert(k.clone(), v) != model.insert(k, v) {
                        return Err(long_fail(o, "mismatch", "insert", at("run")));
                    }
                }
            }
            LongOp::Remove(n) => {
                let k = long_key(c, *n);
                let (ours, want) = (map.remove(&k), model.remove(&k));
                if ours != want {
                    return Err(long_fail(o, "mismatch", "remove", format!("{} {}: ours {ours:?}, std {want:?}", at("remove"), short(&k))));
                }
            }
            LongOp::Get(n) => {
                let k = long_key(c, *n);
                let (ours, want) = (map.get(&k), model.get(&k));
                if ours != want {
                    return Err(long_fail(o, "mismatch", "get", format!("{} {}: ours {ours:?}, std {want:?}", at("get"), short(&k))));
                }
                if map.get(&k[..]) != want {
                    return Err(long_fail(o, "mismatch", "get_borrowed", at("get by &[u8]")));
                }
            }
            LongOp::Range { lo, hi, walk } => {
                let (lo, hi) = (long_key(c, *lo), long_key(c, *hi));
                let mut ours = map.range(lo.clone()..=hi.clone());
                let mut want = model.range(lo..=hi);
                for (step, &front) in walk.iter().chain(std::iter::repeat(&true).take(2000)).enumerate() {
                    let (x, y) = if front { (ours.next(), want.next()) } else { (ours.next_back(), want.next_back()) };
                    if x != y {
                        let f = |e: Option<(&Vec<u8>, &u64)>| e.map(|(k, v)| (short(k), *v));
                        return Err(long_fail(o, "walk", "range", format!("{} step {step}: ours {:?}, std {:?}", at("range"), f(x), f(y))));
                    }
                    if y.is_none() {
                        break;
                    }
                }
            }
            LongOp::Pop(last) => {
                let (ours, want) = if *last { (map.pop_last(), model.pop_last()) } else { (map.pop_first(), model.pop_first()) };
                if ours != want {
                    return Err(long_fail(o, "mismatch", "pop", at("pop")));
                }
            }
            LongOp::SplitAppend(n) => {
                let k = long_key(c, *n);
                let mut ours = map.split_off(&k);
                let mut want = model.split_off(&k);
                if !ours.iter().eq(want.iter()) || !map.iter().eq(model.iter()) {
                    return Err(long_fail(o, "mismatch", "split_off", at("split_off")));
                }
                #[cfg(fuzzing)]
                long_invariants(&ours, &at("split part"))?;
                map.append(&mut ours);
                model.append(&mut want);
            }
        }
        if map.len() != model.len() || !map.iter().eq(model.iter()) {
            return Err(long_fail(o, "mismatch", "whole", format!("{}: {} entries, std {}", at("after"), map.len(), model.len())));
        }
        if map.first_key_value() != model.first_key_value() || map.last_key_value() != model.last_key_value() {
            return Err(long_fail(o, "mismatch", "ends", at("first/last")));
        }
        #[cfg(fuzzing)]
        long_invariants(&map, &at("after"))?;
    }
    ephyra::classify(c.a >= 65_535 || c.a + 1 + c.b >= 65_535, "past_prefix_cap");
    Ok(())
}

#[cfg(fuzzing)]
fn long_invariants<S: InlineKeyStrategy<Vec<u8>>>(map: &bbtree::unsync::BBTreeMap<Vec<u8>, u64, S>, at: &str) -> Result<(), Fail> {
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        map.check_invariants();
        map.check_prefix_lens();
    }));
    r.map_err(|p| {
        let msg = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()));
        long_fail("long", "invariant", "check", format!("{at}: {msg:?}"))
    })
}

/// The shared map from one thread against std, same keys.
pub fn long_sync<S: InlineKeyStrategy<Vec<u8>> + Default + Send + Sync>(c: &LongCase) -> Result<(), Fail> {
    use std::collections::BTreeMap;
    let mut map = bbtree::BBTreeMap::<Vec<u8>, u64, S>::with_strategy(S::default());
    let mut model = BTreeMap::<Vec<u8>, u64>::new();
    let o = "longsync";
    for (i, op) in c.ops.iter().enumerate() {
        let at = |what: &str| format!("op {i} {what}");
        match op {
            LongOp::Insert(n, v) => {
                let k = long_key(c, *n);
                let want = model.insert(k.clone(), *v);
                let ours = map.insert(k, *v).map_err(|_| long_fail(o, "refused", "insert", at("insert")))?;
                if ours != want {
                    return Err(long_fail(o, "mismatch", "insert", format!("{}: ours {ours:?}, std {want:?}", at("insert"))));
                }
            }
            LongOp::Run { start, step, n } => {
                let keys: Vec<Vec<u8>> = (0..*n as u16).map(|j| long_key(c, start.wrapping_add(j.wrapping_mul(*step as u16)))).collect();
                for (j, k) in keys.iter().enumerate() {
                    model.insert(k.clone(), j as u64);
                }
                for (j, k) in keys.into_iter().enumerate() {
                    if map.insert(k, j as u64).is_err() {
                        return Err(long_fail(o, "refused", "insert", at("run")));
                    }
                }
            }
            LongOp::Remove(n) => {
                let k = long_key(c, *n);
                let (ours, want) = (map.remove(&k), model.remove(&k));
                if ours != want {
                    return Err(long_fail(o, "mismatch", "remove", format!("{} {}: ours {ours:?}, std {want:?}", at("remove"), short(&k))));
                }
            }
            LongOp::Get(n) => {
                let k = long_key(c, *n);
                let (ours, want) = (map.get_copied(&k), model.get(&k).copied());
                if ours != want {
                    return Err(long_fail(o, "mismatch", "get", format!("{} {}: ours {ours:?}, std {want:?}", at("get"), short(&k))));
                }
                if map.contains_key(&k[..]) != want.is_some() {
                    return Err(long_fail(o, "mismatch", "contains_key", at("contains_key by &[u8]")));
                }
            }
            LongOp::Range { lo, hi, .. } => {
                let (lo, hi) = (long_key(c, *lo), long_key(c, *hi));
                let mut ours = Vec::new();
                map.scan(lo.clone()..=hi.clone(), |k: &Vec<u8>, v: &u64| {
                    ours.push((k.clone(), *v));
                    std::ops::ControlFlow::<()>::Continue(())
                });
                let want: Vec<(Vec<u8>, u64)> = model.range(lo..=hi).map(|(k, v)| (k.clone(), *v)).collect();
                if ours != want {
                    return Err(long_fail(o, "walk", "scan", format!("{}: {} entries, std {}", at("scan"), ours.len(), want.len())));
                }
            }
            LongOp::Pop(last) => {
                let k = if *last { model.keys().next_back().cloned() } else { model.keys().next().cloned() };
                if let Some(k) = k {
                    let (ours, want) = (map.remove(&k), model.remove(&k));
                    if ours != want {
                        return Err(long_fail(o, "mismatch", "remove", at("remove an end")));
                    }
                }
            }
            LongOp::SplitAppend(_) => {}
        }
        let mut count = 0usize;
        map.scan::<Vec<u8>, ()>(.., |_: &Vec<u8>, _: &u64| {
            count += 1;
            std::ops::ControlFlow::<()>::Continue(())
        });
        if count != model.len() {
            return Err(long_fail(o, "mismatch", "len", format!("{}: {count} entries scanned, std {}", at("after"), model.len())));
        }
    }
    let mut all = Vec::new();
    map.scan::<Vec<u8>, ()>(.., |k: &Vec<u8>, v: &u64| {
        all.push((k.clone(), *v));
        std::ops::ControlFlow::<()>::Continue(())
    });
    if !all.iter().map(|(k, v)| (k, v)).eq(model.iter()) {
        return Err(long_fail(o, "mismatch", "whole", format!("{} entries, std {}", all.len(), model.len())));
    }
    #[cfg(fuzzing)]
    {
        let len = model.len();
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| map.check_invariants(len)));
        if let Err(p) = r {
            let msg = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()));
            return Err(long_fail(o, "invariant", "check", format!("{msg:?}")));
        }
    }
    Ok(())
}
