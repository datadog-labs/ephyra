// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! `bbtree-store`: a store answers every lookup, scan and range as std's
//! `BTreeMap<Vec<u8>, Vec<u8>>` given the same inserts and removes does,
//! at several node configs; an insert the allocator refuses leaves the
//! store holding what it held; dropping the store gives back every page.

use std::alloc::Layout;
use std::collections::BTreeMap;
use std::ops::{Bound, ControlFlow};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use bbtree_properties::range_panics;
use bbtree_store::bbtree::{AllocError, Allocator, Global, NodeConfig};
use bbtree_store::{BBTreeStore, InsertError};
use ephyra::{BoxGen, Config, GenExt, Report, TestResult, check, classify, gens};

/// Pages out, and how many more the allocator gives before refusing
/// (`usize::MAX` for any number). Shared by clones, as the store clones it.
#[derive(Clone, Default)]
struct Budget(Arc<BudgetState>);

#[derive(Default)]
struct BudgetState {
    out: AtomicUsize,
    left: AtomicUsize,
}

impl Budget {
    fn new() -> Self {
        let budget = Budget::default();
        budget.0.left.store(usize::MAX, Ordering::Relaxed);
        budget
    }

    fn take(&self, layout: Layout) -> Result<NonNull<u8>, AllocError> {
        let left = self.0.left.load(Ordering::Relaxed);
        if left == 0 {
            return Err(AllocError);
        }
        if left != usize::MAX {
            self.0.left.store(left - 1, Ordering::Relaxed);
        }
        self.0.out.fetch_add(1, Ordering::Relaxed);
        Global.allocate_zeroed(layout)
    }

    /// Scribbles over a page given back, so that a read after free reads
    /// garbage rather than the old record.
    unsafe fn give(&self, page: NonNull<u8>, layout: Layout) {
        self.0.out.fetch_sub(1, Ordering::Relaxed);
        unsafe {
            page.write_bytes(0xA5, layout.size());
            Global.deallocate(page, layout);
        }
    }
}

unsafe impl Allocator for Budget {
    fn allocate_zeroed(&self, layout: Layout) -> Result<NonNull<u8>, AllocError> {
        self.take(layout)
    }
    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        unsafe { self.give(ptr, layout) }
    }
}

/// Small pages: more levels, more records per heap page boundary.
struct Small;
impl NodeConfig for Small {
    const INTERNAL_PAGE_SIZE: usize = 64;
    const PLAIN_INTERNAL_PAGE_SIZE: usize = 64;
}

/// Heap pages larger than the leaves.
struct BigHeap;
impl NodeConfig for BigHeap {
    const HEAP_PAGE_SIZE: usize = 8192;
}

/// Large leaves, heap pages their size.
struct BigLeaves;
impl NodeConfig for BigLeaves {
    const LEAF_PAGE_SIZE: usize = 4096;
}

#[derive(Clone, Debug, PartialEq)]
enum Op {
    Insert(Vec<u8>, ValueSpec),
    Remove(Vec<u8>),
    Get(Vec<u8>),
    Scan { start: Bound<Vec<u8>>, end: Bound<Vec<u8>>, limit: usize, rev: bool },
    Range { start: Bound<Vec<u8>>, end: Bound<Vec<u8>>, walk: Vec<bool> },
    /// From now on the allocator gives this many more pages.
    Budget(usize),
    /// And then any number again.
    Unlimited,
}

/// A value: its length and a fill seed, so that long values print short.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ValueSpec {
    len: usize,
    seed: u8,
}

impl ValueSpec {
    fn bytes(self) -> Vec<u8> {
        (0..self.len).map(|i| (i as u8).wrapping_mul(31).wrapping_add(self.seed)).collect()
    }
}

/// Keys from a small alphabet, sharing prefixes; lengths around the 8-byte
/// inline key and up to past a quarter of a heap page.
fn key() -> BoxGen<Vec<u8>> {
    let alphabet = gens::pick_from(vec![0u8, 1, b'a', b'b', 0x7f, 0xff]);
    gens::frequency(vec![
        (6, gens::vec(alphabet.clone()).len(0..=10).boxed()),
        (2, gens::vec(alphabet.clone()).len(0..=40).boxed()),
        (1, (gens::vec(alphabet).len(0..=3), gens::int(200usize..=2100), gens::int(0u8..=255))
            .iso(
                |(mut head, len, fill): (Vec<u8>, usize, u8)| {
                    head.resize(len.max(head.len()), fill);
                    head
                },
                |k: &Vec<u8>| (k[..k.len().min(3)].to_vec(), k.len(), k.last().copied().unwrap_or(0)),
            )
            .boxed()),
    ])
    .boxed()
}

fn value() -> BoxGen<ValueSpec> {
    let len = gens::frequency(vec![
        (6, gens::int(0usize..=24).boxed()),
        (3, gens::int(0usize..=400).boxed()),
        (2, gens::int(700usize..=1100).boxed()),
        (1, gens::int(1100usize..=20_000).boxed()),
        (1, gens::pick_from(vec![255usize, 256, 1023, 1024, 1025, 2047, 2048, 4096, 8191, 8192, 8193]).boxed()),
    ]);
    (len, gens::int(0u8..=255)).iso(|(len, seed)| ValueSpec { len, seed }, |v: &ValueSpec| (v.len, v.seed)).boxed()
}

fn bound() -> BoxGen<Bound<Vec<u8>>> {
    (gens::int(0u8..=4), key())
        .iso(
            |(kind, k)| match kind {
                0 | 1 => Bound::Included(k),
                2 | 3 => Bound::Excluded(k),
                _ => Bound::Unbounded,
            },
            |b: &Bound<Vec<u8>>| match b {
                Bound::Included(k) => (0, k.clone()),
                Bound::Excluded(k) => (2, k.clone()),
                Bound::Unbounded => (4, Vec::new()),
            },
        )
        .boxed()
}

fn op() -> BoxGen<Op> {
    gens::frequency(vec![
        (10, (key(), value()).map(|(k, v)| Op::Insert(k, v)).boxed()),
        (4, key().map(Op::Remove).boxed()),
        (3, key().map(Op::Get).boxed()),
        (2, (bound(), bound(), gens::int(0usize..=40), gens::bool())
            .map(|(start, end, limit, rev)| Op::Scan { start, end, limit, rev })
            .boxed()),
        (2, (bound(), bound(), gens::vec(gens::bool()).len(0..=40))
            .map(|(start, end, walk)| Op::Range { start, end, walk })
            .boxed()),
        (1, gens::int(0usize..=12).map(Op::Budget).boxed()),
        (1, gens::constant(Op::Unlimited).boxed()),
    ])
    .boxed()
}

/// A prefix of bulk inserts (to reach several levels) and then ops.
fn case() -> BoxGen<(usize, u8, Vec<Op>)> {
    (
        gens::frequency(vec![(3, gens::int(0usize..=0).boxed()), (2, gens::int(0usize..=3000).boxed())]),
        gens::int(0u8..=255),
        gens::vec(op()).len(0..=120),
    )
        .boxed()
}

fn bulk_key(i: usize, salt: u8) -> Vec<u8> {
    let mut k = (i as u32).wrapping_mul(2654435761).to_be_bytes().to_vec();
    k.truncate(1 + (i + salt as usize) % 4);
    k.extend(std::iter::repeat_n(salt, (i * 7 + salt as usize) % 13));
    k
}

fn run_case<C: NodeConfig>(config: C, bulk: usize, salt: u8, ops: &[Op]) -> TestResult {
    let budget = Budget::new();
    let failure = {
        let store = BBTreeStore::with_config_in(config, budget.clone());
        let mut model: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
        let max_key = store.max_key_len();
        for i in 0..bulk {
            let (k, v) = (bulk_key(i, salt), ValueSpec { len: (i * 37) % 300, seed: salt }.bytes());
            store.insert(&k, &v).expect("unlimited budget");
            model.insert(k, v);
        }
        let mut failure = None;
        for (step, op) in ops.iter().enumerate() {
            let fail = |why: String| Some(format!("op {step} {op:?}: {why}"));
            failure = match op {
                Op::Insert(k, v) => {
                    let v = v.bytes();
                    let got = store.insert(k, &v);
                    match got {
                        Err(InsertError::KeyTooLong) if k.len() > max_key => None,
                        Err(InsertError::Alloc(_)) => {
                            classify(true, "alloc refused");
                            // The store holds what it held: checked below.
                            None
                        }
                        Ok(replaced) if k.len() <= max_key => {
                            let had = model.insert(k.clone(), v).is_some();
                            (replaced != had).then(|| format!("insert returned {replaced}, std had the key: {had}"))
                        }
                        other => Some(format!("insert returned {other:?}, key len {} (max {max_key})", k.len())),
                    }
                    .and_then(fail)
                }
                Op::Remove(k) => {
                    let (got, want) = (store.remove(k), model.remove(k).is_some());
                    (got != want).then(|| format!("remove {got}, std {want}")).and_then(fail)
                }
                Op::Get(k) => {
                    let (got, want) = (store.get(k), model.get(k).cloned());
                    let contains = store.contains_key(k);
                    (got != want || contains != want.is_some())
                        .then(|| format!("get {:?} contains {contains}, std {:?}", got.map(|v| v.len()), want.map(|v| v.len())))
                        .and_then(fail)
                }
                Op::Scan { start, end, limit, rev } => {
                    let range = (start.clone(), end.clone());
                    // std checks the bounds only when it has a root; an empty
                    // store may panic or not.
                    if range_panics(start, end) {
                        let ours = catch_unwind(AssertUnwindSafe(|| store.scan(range.clone(), |_, _| ControlFlow::<()>::Continue(())))).is_err();
                        let std = catch_unwind(AssertUnwindSafe(|| model.range(range.clone()).count())).is_err();
                        (ours != std && !model.is_empty()).then(|| format!("scan panics {ours}, std {std}")).and_then(fail)
                    } else {
                        let mut got = Vec::new();
                        let visit = |k: &[u8], v: &[u8]| {
                            if got.len() == *limit {
                                return ControlFlow::Break(());
                            }
                            got.push((k.to_vec(), v.to_vec()));
                            ControlFlow::Continue(())
                        };
                        if *rev { store.scan_rev(range.clone(), visit) } else { store.scan(range.clone(), visit) };
                        let want: Vec<_> = if *rev {
                            model.range(range).rev().take(*limit).map(|(k, v)| (k.clone(), v.clone())).collect()
                        } else {
                            model.range(range).take(*limit).map(|(k, v)| (k.clone(), v.clone())).collect()
                        };
                        (got != want).then(|| differ(&got, &want)).and_then(fail)
                    }
                }
                Op::Range { start, end, walk } => {
                    let range = (start.clone(), end.clone());
                    // std checks the bounds only when it has a root; an empty
                    // store may panic or not.
                    if range_panics(start, end) {
                        let ours = catch_unwind(AssertUnwindSafe(|| store.range(range.clone()).count())).is_err();
                        let std = catch_unwind(AssertUnwindSafe(|| model.range(range.clone()).count())).is_err();
                        (ours != std && !model.is_empty()).then(|| format!("range panics {ours}, std {std}")).and_then(fail)
                    } else {
                        let mut ours = store.range(range.clone());
                        let mut want = model.range(range).map(|(k, v)| (k.clone(), v.clone()));
                        let mut why = None;
                        for (i, &front) in walk.iter().enumerate() {
                            let (a, b) = if front { (ours.next(), want.next()) } else { (ours.next_back(), want.next_back()) };
                            if a != b {
                                why = Some(format!(
                                    "walk step {i} ({}): store {:?}, std {:?}",
                                    if front { "next" } else { "next_back" },
                                    a.map(|e| (e.0, e.1.len())),
                                    b.map(|e| (e.0, e.1.len()))
                                ));
                                break;
                            }
                        }
                        if why.is_none() {
                            let (rest_ours, rest_want): (Vec<_>, Vec<_>) = (ours.collect(), want.collect());
                            if rest_ours != rest_want {
                                why = Some(format!("after the walk: {}", differ(&rest_ours, &rest_want)));
                            }
                        }
                        why.and_then(fail)
                    }
                }
                Op::Budget(pages) => {
                    budget.0.left.store(*pages, Ordering::Relaxed);
                    None
                }
                Op::Unlimited => {
                    budget.0.left.store(usize::MAX, Ordering::Relaxed);
                    None
                }
            };
            if failure.is_some() {
                break;
            }
            // After a refused insert, and every so often, the whole store.
            if matches!(op, Op::Insert(..)) && (step % 16 == 0 || budget.0.left.load(Ordering::Relaxed) != usize::MAX) {
                let all: Vec<_> = store.range::<[u8], _>(..).collect();
                let want: Vec<_> = model.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                if all != want {
                    failure = fail(format!("whole store after it: {}", differ(&all, &want)));
                    break;
                }
            }
        }
        if failure.is_none() {
            let all: Vec<_> = store.range::<[u8], _>(..).collect();
            let want: Vec<_> = model.into_iter().collect();
            if all != want {
                failure = Some(format!("whole store at the end: {}", differ(&all, &want)));
            }
        }
        failure
    };
    if let Some(failure) = failure {
        return TestResult::Fail(failure);
    }
    let out = budget.0.out.load(Ordering::Relaxed);
    TestResult::check(out == 0, format!("{out} pages still out after the store was dropped"))
}

/// The first difference of two entry lists, values by length.
fn differ(got: &[(Vec<u8>, Vec<u8>)], want: &[(Vec<u8>, Vec<u8>)]) -> String {
    let at = got.iter().zip(want).position(|(a, b)| a != b).unwrap_or(got.len().min(want.len()));
    let show = |e: Option<&(Vec<u8>, Vec<u8>)>| e.map(|(k, v)| format!("{k:?} => {} bytes", v.len()));
    format!("store {} entries, std {}; at {at}: store {:?}, std {:?}", got.len(), want.len(), show(got.get(at)), show(want.get(at)))
}

fn config(name: &str) -> Config {
    Config::default().name(name).tests(200)
}

fn print(report: Report) -> Report {
    eprintln!("{report}");
    report
}

#[test]
fn store_default_config() {
    print(check(&config("store_default_config"), case(), |(bulk, salt, ops)| {
        run_case(bbtree_store::DefaultConfig, bulk, salt, &ops)
    }))
    .assert_passed();
}

#[test]
fn store_small_internal_pages() {
    print(check(&config("store_small_internal_pages"), case(), |(bulk, salt, ops)| run_case(Small, bulk, salt, &ops)))
        .assert_passed();
}

#[test]
fn store_big_heap_pages() {
    print(check(&config("store_big_heap_pages"), case(), |(bulk, salt, ops)| run_case(BigHeap, bulk, salt, &ops)))
        .assert_passed();
}

#[test]
fn store_big_leaves() {
    print(check(&config("store_big_leaves"), case(), |(bulk, salt, ops)| run_case(BigLeaves, bulk, salt, &ops)))
        .assert_passed();
}
