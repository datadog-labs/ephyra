// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! `bbtree::BBTreeMap` with a leaf annex (`WithLeafAnnex`, `LeafAnnex`) and
//! the operations that hand their closure the leaf they hold
//! (`update_or_insert_with_leaf`, `remove_with_leaf`): the API
//! `bbtree-store` builds its record heap on, which nothing else drives.
//!
//! The annex is a probe. Each one has an id and a count of tokens: a vacant
//! closure that accepts adds one to its leaf's annex, a removal takes one
//! back if its leaf's annex has any; a split hands half to the new leaf, a
//! merge takes the other leaf's, the teardown releases the rest. A registry
//! mirrors every annex, so an annex that is copied, dropped, released
//! twice, or seen with other tokens than it was left with shows
//! (`leaf_annex.rs`: "a new tree's leaf gets `Default`'s; a split gives the
//! new leaf ... `Default`'s, then calls `split`; a merge calls `merge` on
//! the leaf that stays with the one that goes; dropping the map, or
//! draining it, calls `release` on each"). At the end every annex was
//! released or merged, and the tokens released are the tokens left.
//!
//! In the closures (`annexed_leaf.rs`: "The leaf's keys, in order: its
//! head's first ... They stay put while the leaf is held"; "No other thread
//! reads or changes them meanwhile"): the leaf's keys are sorted, hold the
//! key (occupied, removal) or not (vacant), are a run of the map's keys
//! with no key of the map missing between them (and the vacant key fits
//! there: it is the leaf it "goes in"), and do not change while held. With
//! threads, no two closures, splits or merges hold one annex at once, and
//! a thread's own keys in the held leaf are what it wrote.
//!
//! Refusals (`map/with_leaf.rs`: "If the allocator has no page for a node
//! the insert makes, or the closure refuses ... having changed nothing: the
//! map is unchanged, and the entry comes back. A node page is taken before
//! `vacant` runs"): page refusals from a budget and closures that refuse;
//! a vacant closure that accepted means the insert happened.

use std::alloc::Layout;
use std::collections::BTreeMap;
use std::fmt::Debug;
use std::ops::Bound;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Barrier, Mutex, MutexGuard};

use bbtree::{Allocator, BBTreeMap, InlineKeyStrategy, LeafAnnex, WithLeafAnnex};
use ephyra::{BoxGen, GenExt, classify, gens};
use serde::{Deserialize, Serialize};

use crate::common::{Fail, ensure, fail, spawn_in};
use crate::syncalloc::Budget;

// ---------- the probe annex and its registry ----------

#[derive(Clone, Copy, Debug, PartialEq)]
enum Fate {
    Live,
    Merged,
    Released,
    /// Dropped without a merge or a release (allowed only for an annex
    /// that never held a token: a `Default` the map made and let go).
    Dropped,
}

struct Mirror {
    tokens: u64,
    fate: Fate,
    /// Was the right side of a split (so not a fresh `Default` any more).
    split_into: bool,
}

#[derive(Default)]
struct Reg {
    next: u64,
    annexes: BTreeMap<u64, Mirror>,
    minted: u64,
    burned: u64,
    released: u64,
    /// The first broken rule, if any: signature, message.
    errors: Vec<(String, String)>,
}

impl Reg {
    fn error(&mut self, sig: &str, msg: String) {
        if self.errors.len() < 4 {
            self.errors.push((sig.to_string(), msg));
        }
    }

    /// The annex `id` must be live and hold `tokens`, as it was left.
    fn expect(&mut self, id: u64, tokens: u64, at: &str) -> bool {
        match self.annexes.get(&id) {
            Some(m) if m.fate == Fate::Live && m.tokens == tokens => true,
            Some(m) => {
                let msg = format!("{at}: annex {id} holds {tokens} tokens, {:?} with {} as last left", m.fate, m.tokens);
                self.error(&format!("annex__stale__{at}"), msg);
                false
            }
            None => {
                self.error(&format!("annex__stale__{at}"), format!("{at}: annex {id} was never made"));
                false
            }
        }
    }

    fn set(&mut self, id: u64, tokens: u64) {
        if let Some(m) = self.annexes.get_mut(&id) {
            m.tokens = tokens;
        }
    }

    fn live(&self) -> usize {
        self.annexes.values().filter(|m| m.fate == Fate::Live).count()
    }

    fn held(&self) -> u64 {
        self.annexes.values().filter(|m| m.fate == Fate::Live).map(|m| m.tokens).sum()
    }
}

static REG: Mutex<Option<Reg>> = Mutex::new(None);
static VALUES: AtomicIsize = AtomicIsize::new(0);

fn reg() -> MutexGuard<'static, Option<Reg>> {
    REG.lock().unwrap_or_else(|e| e.into_inner())
}

fn with_reg<R>(f: impl FnOnce(&mut Reg) -> R) -> R {
    let mut g = reg();
    f(g.get_or_insert_with(Reg::default))
}

pub struct Probe {
    id: u64,
    tokens: u64,
    busy: AtomicBool,
    consumed: bool,
}

impl Default for Probe {
    fn default() -> Self {
        let id = with_reg(|r| {
            r.next += 1;
            let id = r.next;
            r.annexes.insert(id, Mirror { tokens: 0, fate: Fate::Live, split_into: false });
            id
        });
        Probe { id, tokens: 0, busy: AtomicBool::new(false), consumed: false }
    }
}

impl Probe {
    fn is_busy(&self) -> bool {
        self.busy.load(Ordering::SeqCst)
    }
}

impl LeafAnnex for Probe {
    fn split(&mut self, right: &mut Self) {
        with_reg(|r| {
            if self.is_busy() || right.is_busy() {
                r.error("annex__exclusive__split", format!("split of annex {} into {} while a closure holds one of them", self.id, right.id));
            }
            if !r.expect(self.id, self.tokens, "split") || !r.expect(right.id, right.tokens, "split") {
                return;
            }
            let fresh = right.tokens == 0 && !r.annexes[&right.id].split_into;
            if !fresh {
                r.error("annex__fresh__split", format!("split of annex {} into {}, which is not a fresh Default", self.id, right.id));
                return;
            }
            let moved = self.tokens / 2;
            right.tokens = moved;
            self.tokens -= moved;
            r.set(self.id, self.tokens);
            r.set(right.id, right.tokens);
            if let Some(m) = r.annexes.get_mut(&right.id) {
                m.split_into = true;
            }
        });
    }

    fn merge(&mut self, mut right: Self) {
        with_reg(|r| {
            if self.is_busy() || right.is_busy() {
                r.error("annex__exclusive__merge", format!("merge of annex {} into {} while a closure holds one of them", right.id, self.id));
            }
            if self.id == right.id {
                r.error("annex__stale__merge", format!("annex {} merged into itself", self.id));
                return;
            }
            if !r.expect(self.id, self.tokens, "merge") || !r.expect(right.id, right.tokens, "merge") {
                return;
            }
            self.tokens += right.tokens;
            r.set(self.id, self.tokens);
            if let Some(m) = r.annexes.get_mut(&right.id) {
                m.fate = Fate::Merged;
            }
            right.consumed = true;
        });
    }

    fn release(mut self, _alloc: &impl Allocator) {
        with_reg(|r| {
            if self.is_busy() {
                r.error("annex__exclusive__release", format!("release of annex {} while a closure holds it", self.id));
            }
            if r.expect(self.id, self.tokens, "release") {
                r.released += self.tokens;
                if let Some(m) = r.annexes.get_mut(&self.id) {
                    m.fate = Fate::Released;
                }
            }
            self.consumed = true;
        });
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        if self.consumed {
            return;
        }
        with_reg(|r| {
            let (id, tokens) = (self.id, self.tokens);
            match r.annexes.get_mut(&id) {
                Some(m) if m.fate == Fate::Live => {
                    m.fate = Fate::Dropped;
                    if tokens > 0 || m.tokens > 0 {
                        r.error("annex__lost__drop", format!("annex {id} dropped without a release or a merge, holding {tokens} tokens"));
                    }
                }
                Some(m) => {
                    let fate = m.fate;
                    r.error("annex__double__drop", format!("annex {id} dropped, already {fate:?}"));
                }
                None => r.error("annex__double__drop", format!("annex {id} dropped, never made")),
            }
        });
    }
}

/// Takes the probe for a closure: no one else may hold it.
fn hold(annex: &Probe, at: &str) -> Result<(), (String, String)> {
    if annex.busy.swap(true, Ordering::SeqCst) {
        return Err(("annex__exclusive__closure".into(), format!("{at}: annex {} is held by another closure", annex.id)));
    }
    Ok(())
}

fn unhold(annex: &Probe) {
    annex.busy.store(false, Ordering::SeqCst);
}

// ---------- values ----------

/// A value that counts how many of it are alive.
#[derive(Debug, PartialEq)]
pub struct Val(u64);

impl Val {
    fn new(n: u64) -> Self {
        VALUES.fetch_add(1, Ordering::SeqCst);
        Val(n)
    }
}

impl Drop for Val {
    fn drop(&mut self) {
        VALUES.fetch_sub(1, Ordering::SeqCst);
    }
}

// ---------- keys ----------

pub trait AKey: Ord + Clone + Debug + Send + Sync + 'static {
    fn of(n: u64) -> Self;
    /// The number `of` made it from.
    fn num(&self) -> u64;
}

impl AKey for u64 {
    fn of(n: u64) -> Self {
        n
    }
    fn num(&self) -> u64 {
        *self
    }
}

impl AKey for String {
    /// Zero-padded behind a shared prefix, so the strings sort as the
    /// numbers do; the highest numbers leave the prefix.
    fn of(n: u64) -> Self {
        if n >= 1 << 40 { format!("{n:020}") } else { format!("store/key/{n:012}") }
    }
    fn num(&self) -> u64 {
        self.trim_start_matches("store/key/").parse().unwrap_or(u64::MAX)
    }
}

impl AKey for Vec<u8> {
    /// Every 7th key 260 bytes long (saturated hash tags), the rest short.
    fn of(n: u64) -> Self {
        let mut k = b"bb".to_vec();
        k.extend_from_slice(&n.to_be_bytes());
        if n % 7 == 3 {
            k.resize(260, b'z');
        }
        k
    }
    fn num(&self) -> u64 {
        self.get(2..10).map_or(u64::MAX, |b| u64::from_be_bytes(b.try_into().unwrap_or_default()))
    }
}

fn raw_key() -> BoxGen<u64> {
    gens::frequency(vec![
        (5, gens::int(0u64..=400).boxed()),
        (4, gens::int(0u64..=6000).boxed()),
        (1, gens::int(0u64..=60_000).map(|k| k * 1_000_003).boxed()),
        (1, gens::edges(0u64..=u64::MAX).boxed()),
    ])
    .boxed()
}

// ---------- single thread ----------

/// How the closures answer: 0 and 1 accept, 2 the vacant one refuses, 3
/// the occupied one, 4 both.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Op {
    Put(u64, u64, u8),
    Take(u64),
    TakeRun(u64, u16),
    Insert(u64, u64),
    Remove(u64),
    Get(u64),
    Budget(u8),
    Unlimited,
    Clear,
    Check,
}

fn op() -> BoxGen<Op> {
    let k = raw_key();
    let v = gens::int(0u64..=1_000_000);
    gens::frequency(vec![
        (8, (k.clone(), v.clone(), gens::int(0u8..=4)).map(|(k, v, m)| Op::Put(k, v, m)).boxed()),
        (5, k.clone().map(Op::Take).boxed()),
        (1, (k.clone(), gens::int(1u16..=1500)).map(|(k, n)| Op::TakeRun(k, n)).boxed()),
        (2, (k.clone(), v).map(|(k, v)| Op::Insert(k, v)).boxed()),
        (2, k.clone().map(Op::Remove).boxed()),
        (2, k.map(Op::Get).boxed()),
        (2, gens::int(0u8..=5).map(Op::Budget).boxed()),
        (1, gens::constant(Op::Unlimited).boxed()),
        (1, gens::constant(Op::Clear).boxed()),
        (1, gens::constant(Op::Check).boxed()),
    ])
    .boxed()
}

/// Preload (entries, stride), ops, the end: 0 drop, 1 drain `into_iter`
/// part way, 2 clear then drop.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Case {
    pub bulk: u16,
    pub stride: u8,
    pub ops: Vec<Op>,
    pub end: u8,
    pub drain: u16,
}

pub fn case() -> BoxGen<Case> {
    (
        gens::frequency(vec![(1, gens::int(0u16..=40).boxed()), (2, gens::int(0u16..=4000).boxed())]).boxed(),
        gens::int(1u8..=9),
        gens::vec(op()).len(0..=250),
        gens::int(0u8..=2),
        gens::int(0u16..=3000),
    )
        .map(|(bulk, stride, ops, end, drain)| Case { bulk, stride, ops, end, drain })
        .boxed()
}

type AMap<K, S> = BBTreeMap<K, Val, WithLeafAnnex<S, Probe>, Budget>;

/// Labels: what the case's annexes went through.
fn label_fates() {
    with_reg(|r| {
        classify(r.annexes.values().any(|m| m.split_into), "annex_split");
        classify(r.annexes.values().any(|m| m.fate == Fate::Merged), "annex_merged");
        classify(r.released > 0, "annex_released_tokens");
    });
}

fn start() {
    *reg() = Some(Reg::default());
    VALUES.store(0, Ordering::SeqCst);
}

/// The registry's first error, as a failure.
fn reg_errors(at: &str) -> Result<(), Fail> {
    with_reg(|r| match r.errors.first() {
        Some((sig, msg)) => Err(fail(sig, format!("{at}: {msg}"))),
        None => Ok(()),
    })
}

/// The leaf's keys against the map's (`model`, before the op): sorted, all
/// the map's, a run of them with none missing, and `key` in it (or, for a
/// vacant one, fitting in it).
fn check_leaf<K: Ord + Clone + Debug>(keys: &[K], key: &K, present: bool, model: &BTreeMap<K, u64>) -> Option<String> {
    if !keys.windows(2).all(|w| w[0] < w[1]) {
        return Some(format!("the leaf's keys are not ascending: {keys:?}"));
    }
    if keys.contains(key) != present {
        return Some(format!("key {key:?} {} the leaf's keys {keys:?}", if present { "missing from" } else { "already in" }));
    }
    if let Some(k) = keys.iter().find(|k| !model.contains_key(*k)) {
        return Some(format!("the leaf holds {k:?}, which the map doesn't"));
    }
    let (Some(first), Some(last)) = (keys.first(), keys.last()) else {
        return None;
    };
    let (lo, hi) = if present { (first, last) } else { (first.min(key), last.max(key)) };
    let between = model.range((Bound::Included(lo), Bound::Included(hi))).count();
    if between != keys.len() {
        return Some(format!(
            "the leaf's {} keys {first:?}..={last:?} are not a run of the map's: it holds {between} from {lo:?} to {hi:?}",
            keys.len()
        ));
    }
    None
}

fn invariants<K: AKey, S: InlineKeyStrategy<K>>(map: &mut AMap<K, S>, len: usize, at: &str) -> Result<(), Fail> {
    match catch_unwind(AssertUnwindSafe(|| map.check_invariants(len))) {
        Ok(()) => Ok(()),
        Err(p) => {
            let msg = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()));
            Err(fail("annex__invariant__check", format!("{at}: {}", msg.unwrap_or_default())))
        }
    }
}

/// What a closure saw, for the checks after it.
#[derive(Default)]
struct Seen {
    occupied: u32,
    vacant: u32,
    vacant_ok: bool,
    bad: Option<(String, String)>,
}

pub fn run<K: AKey, S: InlineKeyStrategy<K> + Default>(case: &Case) -> Result<(), Fail> {
    start();
    let out = run_inner::<K, S>(case);
    label_fates();
    // Values and pages are checked whatever the outcome: errors first.
    out?;
    reg_errors("after the drop")?;
    let values = VALUES.load(Ordering::SeqCst);
    ensure(values == 0, "annex__leak__values", || format!("after the drop {values} values alive"))?;
    with_reg(|r| {
        let live: Vec<u64> = r.annexes.iter().filter(|(_, m)| m.fate == Fate::Live).map(|(id, _)| *id).collect();
        ensure(live.is_empty(), "annex__leak__drop", || format!("after the drop annexes {live:?} were never released or merged"))?;
        let left = r.minted - r.burned;
        ensure(r.released == left, "annex__tokens__drop", || {
            format!("released {} tokens, {} minted, {} taken back: {left} were left", r.released, r.minted, r.burned)
        })
    })
}

fn run_inner<K: AKey, S: InlineKeyStrategy<K> + Default>(case: &Case) -> Result<(), Fail> {
    let budget = Budget::new();
    let mut map: AMap<K, S> = BBTreeMap::new_in(budget.clone());
    let mut model: BTreeMap<K, u64> = BTreeMap::new();
    let stride = u64::from(case.stride.max(1));
    for i in 0..u64::from(case.bulk) {
        let n = i * stride;
        let k = K::of(n);
        let r = map.update_or_insert_with_leaf(
            k.clone(),
            Val::new(n),
            |_, _, _| -> Result<bool, Layout> { Ok(false) },
            |mut leaf, _, _| {
                let a = leaf.annex();
                a.tokens += 1;
                with_reg(|r| {
                    r.minted += 1;
                    r.set(a.id, a.tokens);
                });
                Ok(true)
            },
        );
        ensure(matches!(r, Ok(true)), "annex__mismatch__bulk", || format!("bulk insert {n}: {:?}", r.as_ref().map_err(|_| "refused")))?;
        model.insert(k, n);
    }
    reg_errors("after the preload")?;
    for (step, op) in case.ops.iter().enumerate() {
        let at = |why: String| format!("op {step} {op:?}: {why}");
        match *op {
            Op::Put(k, v, mode) => {
                let k = K::of(k);
                let seen = std::cell::RefCell::new(Seen::default());
                let before = model.get(&k).copied();
                let r = map.update_or_insert_with_leaf(
                    k.clone(),
                    Val::new(v),
                    |mut leaf, stored, value| {
                        seen.borrow_mut().occupied += 1;
                        let keys: Vec<K> = leaf.keys().cloned().collect();
                        if let Some(why) = check_leaf(&keys, stored, true, &model) {
                            seen.borrow_mut().bad = Some(("annex__keys__occupied".into(), why));
                        }
                        let a = leaf.annex();
                        if let Err(e) = hold(a, "occupied") {
                            seen.borrow_mut().bad = Some(e);
                        }
                        with_reg(|r| r.expect(a.id, a.tokens, "closure"));
                        unhold(a);
                        if mode == 3 || mode == 4 {
                            return Err(Layout::new::<u64>());
                        }
                        let old = value.0;
                        value.0 = v;
                        Ok(Some(old))
                    },
                    |mut leaf, new, _| {
                        seen.borrow_mut().vacant += 1;
                        let keys: Vec<K> = leaf.keys().cloned().collect();
                        if let Some(why) = check_leaf(&keys, new, false, &model) {
                            seen.borrow_mut().bad = Some(("annex__keys__vacant".into(), why));
                        }
                        let a = leaf.annex();
                        if let Err(e) = hold(a, "vacant") {
                            seen.borrow_mut().bad = Some(e);
                        }
                        unhold(a);
                        if mode == 2 || mode == 4 {
                            return Err(Layout::new::<[u64; 8]>());
                        }
                        if with_reg(|r| r.expect(a.id, a.tokens, "closure")) {
                            a.tokens += 1;
                            with_reg(|r| {
                                r.minted += 1;
                                r.set(a.id, a.tokens);
                            });
                        }
                        seen.borrow_mut().vacant_ok = true;
                        Ok(None)
                    },
                );
                let mut seen = seen.into_inner();
                if let Some((sig, why)) = seen.bad.take() {
                    return Err(fail(&sig, at(why)));
                }
                ensure(seen.occupied + seen.vacant <= 1, "annex__closure_ran__put", || {
                    at(format!("occupied ran {} times, vacant {}", seen.occupied, seen.vacant))
                })?;
                ensure(seen.occupied == 0 || before.is_some(), "annex__mismatch__put", || at("occupied ran on a key the map doesn't hold".into()))?;
                ensure(seen.vacant == 0 || before.is_none(), "annex__mismatch__put", || at(format!("vacant ran on a key the map holds ({before:?})")))?;
                match r {
                    Ok(old) => {
                        ensure(seen.occupied + seen.vacant == 1, "annex__closure_ran__put", || at("Ok, and no closure ran".into()))?;
                        ensure(old == before, "annex__mismatch__put", || at(format!("returned {old:?}, std {before:?}")))?;
                        model.insert(k, v);
                    }
                    Err(e) => {
                        classify(true, "refused");
                        ensure(!seen.vacant_ok, "annex__closure_ran__refused", || {
                            at("vacant accepted, and the insert was refused (\"A node page is taken before `vacant` runs\")".into())
                        })?;
                        let (rk, rv) = e.into_entry();
                        ensure(rk == k && rv.0 == v, "annex__refused_entry__put", || at(format!("refused entry came back as ({rk:?}, {rv:?})")))?;
                        let now = map.get_copied_by(&k);
                        ensure(now == before, "annex__refused_changed__put", || at(format!("after a refusal the key holds {now:?}, before it {before:?}")))?;
                    }
                }
            }
            Op::Take(k) => take(&map, &mut model, K::of(k)).map_err(|f| fail(&f.sig, at(f.msg)))?,
            Op::TakeRun(k, n) => {
                let from = K::of(k);
                let keys: Vec<K> = model.range(from..).take(usize::from(n)).map(|(k, _)| k.clone()).collect();
                for k in keys {
                    take(&map, &mut model, k).map_err(|f| fail(&f.sig, at(f.msg)))?;
                }
            }
            Op::Insert(k, v) => {
                let k = K::of(k);
                match map.insert(k.clone(), Val::new(v)) {
                    Ok(old) => {
                        let want = model.insert(k, v);
                        let old = old.map(|o| o.0);
                        ensure(old == want, "annex__mismatch__insert", || at(format!("insert returned {old:?}, std {want:?}")))?;
                    }
                    Err(e) => {
                        classify(true, "refused");
                        let (rk, rv) = e.into_entry();
                        ensure(rk == k && rv.0 == v, "annex__refused_entry__insert", || at(format!("refused entry came back as ({rk:?}, {rv:?})")))?;
                    }
                }
            }
            Op::Remove(k) => {
                let k = K::of(k);
                let got = map.remove(&k).map(|v| v.0);
                let want = model.remove(&k);
                ensure(got == want, "annex__mismatch__remove", || at(format!("remove {got:?}, std {want:?}")))?;
            }
            Op::Get(k) => {
                let k = K::of(k);
                let got = map.get_copied_by(&k);
                let want = model.get(&k).copied();
                ensure(got == want, "annex__mismatch__get", || at(format!("get {got:?}, std {want:?}")))?;
            }
            Op::Budget(n) => budget.set(usize::from(n)),
            Op::Unlimited => budget.set(usize::MAX),
            Op::Clear => {
                let before: Vec<u64> = with_reg(|r| r.annexes.iter().filter(|(_, m)| m.fate == Fate::Live).map(|(id, _)| *id).collect());
                map.clear();
                model.clear();
                reg_errors(&at("clear".into()))?;
                let still: Vec<u64> = with_reg(|r| before.iter().copied().filter(|id| r.annexes[id].fate == Fate::Live).collect());
                ensure(still.is_empty(), "annex__leak__clear", || at(format!("annexes {still:?} live after clear (\"giving every page back\")")))?;
            }
            Op::Check => {
                invariants(&mut map, model.len(), &at("check".into()))?;
                let all: Vec<(K, u64)> = map.range_cloned_by();
                let want: Vec<(K, u64)> = model.iter().map(|(k, v)| (k.clone(), *v)).collect();
                ensure(all == want, "annex__mismatch__whole", || at(format!("the map holds {} entries, std {}", all.len(), want.len())))?;
            }
        }
        reg_errors(&at("after".into()))?;
        let values = VALUES.load(Ordering::SeqCst);
        ensure(values == model.len() as isize, "annex__drop_count__values", || at(format!("{values} values alive, the map holds {}", model.len())))?;
        let (live, out) = (with_reg(|r| r.live()), budget.out());
        ensure(live <= out, "annex__leak__live", || at(format!("{live} annexes live, in {out} pages")))?;
    }
    budget.set(usize::MAX);
    invariants(&mut map, model.len(), "at the end")?;
    let held = with_reg(|r| r.held());
    let left = with_reg(|r| r.minted - r.burned - r.released);
    ensure(held == left, "annex__tokens__end", || format!("at the end the annexes hold {held} tokens, {left} left"))?;
    match case.end {
        1 => {
            let mut drained = map.into_iter();
            for (i, (wk, wv)) in model.iter().take(usize::from(case.drain)).enumerate() {
                let got = drained.next().map(|(k, v)| (k, v.0));
                ensure(got.as_ref() == Some(&(wk.clone(), *wv)), "annex__mismatch__into_iter", || {
                    format!("into_iter item {i}: {got:?}, std {:?}", (wk, wv))
                })?;
            }
            drop(drained);
        }
        2 => {
            map.clear();
            reg_errors("at the end's clear")?;
            drop(map);
        }
        _ => drop(map),
    }
    let out = budget.out();
    ensure(out == 0, "annex__leak__pages", || format!("after the drop {out} pages out"))
}

/// `remove_with_leaf`, taking a token back from the leaf's annex.
fn take<K: AKey, S: InlineKeyStrategy<K>>(map: &AMap<K, S>, model: &mut BTreeMap<K, u64>, k: K) -> Result<(), Fail> {
    let mut ran = 0;
    let mut bad = None;
    let got = map.remove_with_leaf(&k, |mut leaf, stored, value| {
        ran += 1;
        let keys: Vec<K> = leaf.keys().cloned().collect();
        if let Some(why) = check_leaf(&keys, stored, true, model) {
            bad = Some(("annex__keys__remove".to_string(), why));
        }
        if model.get(stored) != Some(&value.0) {
            bad = Some(("annex__mismatch__take".to_string(), format!("f saw ({stored:?}, {}), std {:?}", value.0, model.get(stored))));
        }
        let a = leaf.annex();
        if let Err(e) = hold(a, "remove") {
            bad = Some(e);
        }
        unhold(a);
        if a.tokens > 0 && with_reg(|r| r.expect(a.id, a.tokens, "closure")) {
            a.tokens -= 1;
            with_reg(|r| {
                r.burned += 1;
                r.set(a.id, a.tokens);
            });
        }
    });
    if let Some((sig, why)) = bad {
        return Err(fail(&sig, why));
    }
    let want = model.remove(&k);
    let got_kv = got.as_ref().map(|(k, v)| (k.clone(), v.0));
    ensure(got_kv == want.map(|v| (k.clone(), v)), "annex__mismatch__take", || format!("remove_with_leaf {got_kv:?}, std {want:?}"))?;
    ensure(ran == u32::from(want.is_some()), "annex__closure_ran__take", || format!("f ran {ran} times, the map held {want:?}"))
}

/// Reads through the sync map's API for `Val`, which isn't `Clone`.
trait ReadBy<K> {
    fn get_copied_by(&self, k: &K) -> Option<u64>;
    fn range_cloned_by(&self) -> Vec<(K, u64)>;
}

impl<K: AKey, S: InlineKeyStrategy<K>> ReadBy<K> for AMap<K, S> {
    fn get_copied_by(&self, k: &K) -> Option<u64> {
        self.read(k, |_, v| v.0)
    }
    fn range_cloned_by(&self) -> Vec<(K, u64)> {
        let mut all = Vec::new();
        self.scan::<K, _>(.., |k, v| {
            all.push((k.clone(), v.0));
            std::ops::ControlFlow::<()>::Continue(())
        });
        all
    }
}

// ---------- threads ----------

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum TOp {
    Put(u32, u8),
    Take(u32),
    TakeRun(u32, u16),
    Insert(u32),
    Remove(u32),
}

fn top() -> BoxGen<TOp> {
    let k = gens::frequency(vec![(3, gens::int(0u32..=300).boxed()), (2, gens::int(0u32..=5000).boxed())]).boxed();
    gens::frequency(vec![
        (6, (k.clone(), gens::int(0u8..=4)).map(|(k, m)| TOp::Put(k, m)).boxed()),
        (4, k.clone().map(TOp::Take).boxed()),
        (1, (k.clone(), gens::int(1u16..=400)).map(|(k, n)| TOp::TakeRun(k, n)).boxed()),
        (2, k.clone().map(TOp::Insert).boxed()),
        (2, k.map(TOp::Remove).boxed()),
    ])
    .boxed()
}

/// Preload per thread, spin while held, one plan per thread (2 to 4).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TCase {
    pub bulk: u16,
    pub spin: u16,
    pub plans: Vec<Vec<TOp>>,
}

pub fn tcase() -> BoxGen<TCase> {
    (gens::int(0u16..=3000), gens::int(0u16..=300), gens::vec(gens::vec(top()).len(1..=400)).len(2..=4))
        .map(|(bulk, spin, plans)| TCase { bulk, spin, plans })
        .boxed()
}

/// Thread `t`'s key `n` of `threads`: neighbouring keys belong to different
/// threads, so their closures meet in one leaf.
fn tkey<K: AKey>(n: u32, t: usize, threads: usize) -> K {
    K::of(u64::from(n) * threads as u64 + t as u64)
}

/// In a closure: the leaf's keys are sorted and hold `key` (or not), stay
/// put while held, and this thread's keys among them are its own.
fn check_held<K: AKey>(keys_now: impl Fn() -> Vec<K>, key: &K, present: bool, mine: &BTreeMap<K, u64>, is_mine: impl Fn(&K) -> bool, spin: u16) -> Option<(String, String)> {
    let keys = keys_now();
    if !keys.windows(2).all(|w| w[0] < w[1]) {
        return Some(("annex__keys__threads".into(), format!("the leaf's keys are not ascending: {keys:?}")));
    }
    if keys.contains(key) != present {
        return Some(("annex__keys__threads".into(), format!("key {key:?} {} the held leaf's keys", if present { "missing from" } else { "already in" })));
    }
    if let (Some(first), Some(last)) = (keys.first(), keys.last()) {
        let own: Vec<&K> = keys.iter().filter(|k| is_mine(k)).collect();
        let want: Vec<&K> = mine.range((Bound::Included(first), Bound::Included(last))).map(|(k, _)| k).collect();
        if own != want {
            return Some((
                "annex__keys__threads".into(),
                format!("the held leaf {first:?}..={last:?} holds {} of this thread's keys, it wrote {}", own.len(), want.len()),
            ));
        }
    }
    // No yield_now while the latch is held: a holder that yields hands its CPU to the
    // threads spinning on its latch (bbtree latches spin, never park), and under the
    // search's oversubscription that convoy stretched 4% of cases from 0.02 s to seconds,
    // past the 30 s deadline (crash__hang__annex, windows 17 and 19).
    for _ in 0..spin {
        std::hint::spin_loop();
    }
    let again = keys_now();
    if again != keys {
        return Some(("annex__moved__keys".into(), format!("the held leaf's keys changed: {} keys, then {}", keys.len(), again.len())));
    }
    None
}

pub fn run_threads<K: AKey, S: InlineKeyStrategy<K> + Default>(case: &TCase) -> Result<(), Fail> {
    start();
    let out = run_threads_inner::<K, S>(case);
    label_fates();
    out?;
    reg_errors("after the drop")?;
    let values = VALUES.load(Ordering::SeqCst);
    ensure(values == 0, "annex__leak__values", || format!("after the drop {values} values alive"))?;
    with_reg(|r| {
        let live = r.live();
        ensure(live == 0, "annex__leak__drop", || format!("after the drop {live} annexes were never released or merged"))?;
        let left = r.minted - r.burned;
        ensure(r.released == left, "annex__tokens__drop", || {
            format!("released {} tokens, {} minted, {} taken back: {left} were left", r.released, r.minted, r.burned)
        })
    })
}

fn run_threads_inner<K: AKey, S: InlineKeyStrategy<K> + Default>(case: &TCase) -> Result<(), Fail> {
    let threads = case.plans.len().max(1);
    let budget = Budget::new();
    let mut map: AMap<K, S> = BBTreeMap::new_in(budget.clone());
    let mut models: Vec<BTreeMap<K, u64>> = vec![BTreeMap::new(); threads];
    for n in 0..u32::from(case.bulk) {
        let t = n as usize % threads;
        let k = tkey::<K>(n, t, threads);
        let _ = map.insert(k.clone(), Val::new(u64::from(n)));
        models[t].insert(k, u64::from(n));
    }
    let barrier = Barrier::new(threads);
    let results: Vec<Result<BTreeMap<K, u64>, Fail>> = std::thread::scope(|s| {
        let handles: Vec<_> = case
            .plans
            .iter()
            .enumerate()
            .map(|(t, plan)| {
                let (map, barrier) = (&map, &barrier);
                let mut mine = models[t].clone();
                let spin = case.spin;
                spawn_in(s, move || -> Result<BTreeMap<K, u64>, Fail> {
                    barrier.wait();
                    let is_mine = |k: &K| -> bool { k.num() % threads as u64 == t as u64 };
                    for (step, op) in plan.iter().enumerate() {
                        let at = |why: String| format!("thread {t} op {step} {op:?}: {why}");
                        let do_take = |k: K, mine: &mut BTreeMap<K, u64>| -> Result<(), Fail> {
                            let mut bad = None;
                            let got = map.remove_with_leaf(&k, |mut leaf, stored, _| {
                                let a = leaf.annex();
                                if let Err(e) = hold(a, "remove") {
                                    bad = Some(e);
                                    return;
                                }
                                let id_tokens = (a.id, a.tokens);
                                let leaf_ro = &leaf;
                                if let Some(e) = check_held(|| leaf_ro.keys().cloned().collect(), stored, true, mine, &is_mine, spin) {
                                    bad = Some(e);
                                }
                                let a = leaf.annex();
                                if a.tokens > 0 && with_reg(|r| r.expect(id_tokens.0, id_tokens.1, "closure")) {
                                    a.tokens -= 1;
                                    with_reg(|r| {
                                        r.burned += 1;
                                        r.set(a.id, a.tokens);
                                    });
                                }
                                unhold(a);
                            });
                            if let Some((sig, why)) = bad {
                                return Err(fail(&sig, at(why)));
                            }
                            let want = mine.remove(&k);
                            let got = got.map(|(_, v)| v.0);
                            ensure(got == want, "annex__mismatch__threads", || at(format!("remove_with_leaf {got:?}, its own model {want:?}")))
                        };
                        match *op {
                            TOp::Put(n, mode) => {
                                let k = tkey::<K>(n, t, threads);
                                let v = u64::from(n) + step as u64 * 100_000;
                                let before = mine.get(&k).copied();
                                let bad = std::cell::RefCell::new(None);
                                let vacant_ok = std::cell::Cell::new(false);
                                let r = map.update_or_insert_with_leaf(
                                    k.clone(),
                                    Val::new(v),
                                    |mut leaf, stored, value| {
                                        if let Err(e) = hold(leaf.annex(), "occupied") {
                                            *bad.borrow_mut() = Some(e);
                                            return Err(Layout::new::<u64>());
                                        }
                                        let leaf_ro = &leaf;
                                        if let Some(e) = check_held(|| leaf_ro.keys().cloned().collect(), stored, true, &mine, &is_mine, spin) {
                                            *bad.borrow_mut() = Some(e);
                                        }
                                        unhold(leaf.annex());
                                        if mode >= 3 {
                                            return Err(Layout::new::<u64>());
                                        }
                                        let old = value.0;
                                        value.0 = v;
                                        Ok(Some(old))
                                    },
                                    |mut leaf, new, _| {
                                        if let Err(e) = hold(leaf.annex(), "vacant") {
                                            *bad.borrow_mut() = Some(e);
                                            return Err(Layout::new::<u64>());
                                        }
                                        let leaf_ro = &leaf;
                                        if let Some(e) = check_held(|| leaf_ro.keys().cloned().collect(), new, false, &mine, &is_mine, spin) {
                                            *bad.borrow_mut() = Some(e);
                                        }
                                        let a = leaf.annex();
                                        if mode != 2 && mode != 4 && with_reg(|r| r.expect(a.id, a.tokens, "closure")) {
                                            a.tokens += 1;
                                            with_reg(|r| {
                                                r.minted += 1;
                                                r.set(a.id, a.tokens);
                                            });
                                            vacant_ok.set(true);
                                        }
                                        unhold(a);
                                        if mode == 2 || mode == 4 { Err(Layout::new::<[u64; 8]>()) } else { Ok(None) }
                                    },
                                );
                                if let Some((sig, why)) = bad.into_inner() {
                                    return Err(fail(&sig, at(why)));
                                }
                                match r {
                                    Ok(old) => {
                                        ensure(old == before, "annex__mismatch__threads", || at(format!("returned {old:?}, its own model {before:?}")))?;
                                        mine.insert(k, v);
                                    }
                                    Err(e) => {
                                        ensure(!vacant_ok.get(), "annex__closure_ran__refused", || at("vacant accepted, and the insert was refused".into()))?;
                                        let (rk, _) = e.into_entry();
                                        ensure(rk == k, "annex__refused_entry__threads", || at(format!("refused entry came back as {rk:?}")))?;
                                    }
                                }
                            }
                            TOp::Take(n) => do_take(tkey::<K>(n, t, threads), &mut mine)?,
                            TOp::TakeRun(n, len) => {
                                let from = tkey::<K>(n, t, threads);
                                let keys: Vec<K> = mine.range(from..).take(usize::from(len)).map(|(k, _)| k.clone()).collect();
                                for k in keys {
                                    do_take(k, &mut mine)?;
                                }
                            }
                            TOp::Insert(n) => {
                                let k = tkey::<K>(n, t, threads);
                                let v = u64::from(n) + step as u64 * 100_000;
                                let old = map.insert(k.clone(), Val::new(v)).map_err(|_| fail("annex__error__threads", at("insert refused, no budget".into())))?;
                                let want = mine.insert(k, v);
                                let old = old.map(|o| o.0);
                                ensure(old == want, "annex__mismatch__threads", || at(format!("insert returned {old:?}, its own model {want:?}")))?;
                            }
                            TOp::Remove(n) => {
                                let k = tkey::<K>(n, t, threads);
                                let got = map.remove(&k).map(|v| v.0);
                                let want = mine.remove(&k);
                                ensure(got == want, "annex__mismatch__threads", || at(format!("remove {got:?}, its own model {want:?}")))?;
                            }
                        }
                    }
                    Ok(mine)
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap_or_else(|_| Err(fail("crash__panic__annex", "a thread panicked")))).collect()
    });
    let mut model = BTreeMap::new();
    for r in results {
        model.extend(r?);
    }
    reg_errors("after the join")?;
    invariants(&mut map, model.len(), "after the join")?;
    let all = map.range_cloned_by();
    let want: Vec<(K, u64)> = model.iter().map(|(k, v)| (k.clone(), *v)).collect();
    ensure(all == want, "annex__mismatch__whole", || format!("after the join the map holds {} entries, the threads wrote {}", all.len(), want.len()))?;
    let values = VALUES.load(Ordering::SeqCst);
    ensure(values == model.len() as isize, "annex__drop_count__values", || format!("after the join {values} values alive, the map holds {}", model.len()))?;
    let (held, left) = with_reg(|r| (r.held(), r.minted - r.burned - r.released));
    ensure(held == left, "annex__tokens__end", || format!("after the join the annexes hold {held} tokens, {left} left"))?;
    drop(map);
    let out = budget.out();
    ensure(out == 0, "annex__leak__pages", || format!("after the drop {out} pages out"))
}
