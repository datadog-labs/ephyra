// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! A stateful model of the single-threaded map and set over the API the
//! other properties never call: the entry API (`entry`, `first_entry`,
//! `last_entry` and every entry method), mutable walks (`iter_mut`,
//! `values_mut`, `range_mut`), consuming walks (`into_iter`, `into_keys`,
//! `into_values`, dropped part way), `Clone` (also with a value whose clone
//! panics part way), `Extend`, `get_many`/`insert_many`, comparisons, and
//! all of `BBTreeSet`: set algebra (`difference`, `intersection`, `union`,
//! `symmetric_difference`, and the operators that build sets), relations
//! (`is_subset`, `is_superset`, `is_disjoint`), split/append/retain/
//! extract_if on sets.
//!
//! Two maps and two sets, each against std's `BTreeMap`/`BTreeSet`
//! ("drop-in replacement"). After every step: whole contents, the tree's
//! own invariant checks (`--cfg fuzzing`), and the number of live values
//! (values count themselves, so a leak or a double drop shows). At the end
//! every collection is dropped and the pages out must be 0. Every walk
//! checks `size_hint` against what is left (an iterator's contract:
//! lower <= remaining <= upper; exact for `ExactSizeIterator`s), and that
//! a finished iterator stays finished.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;
use std::alloc::Layout;
use std::ops::Bound;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::NonNull;

use bbtree::unsync::{BBTreeMap, BBTreeSet};
use bbtree::unsync::bbtree_map::Entry;
use bbtree::{AllocError, Allocator, Global, InlineKeyStrategy};
use ephyra::{BoxGen, GenExt, classify, gens};
use serde::{Deserialize, Serialize};

use crate::common::{Fail, MapSeed, RawBound, ensure, fail, range_panics, raw_key};

thread_local! {
    static LIVE: Cell<i64> = const { Cell::new(0) };
    static PAGES: Cell<i64> = const { Cell::new(0) };
    /// Clones left before one panics; 0 is off.
    static CLONE_BOMB: Cell<u64> = const { Cell::new(0) };
    /// The tag keys made now get (`Tagged`, `TagU64`): the op's number + 1,
    /// 0 for the preload, so a key's tag tells which op made it.
    static TAG: Cell<u32> = const { Cell::new(0) };
    /// Whether the key type carries a tag, so that results compare by
    /// `Debug` too: equal keys of different ops are told apart.
    static TAGGED: Cell<bool> = const { Cell::new(false) };
}

/// `a == b`, and for tagged keys also the same tags (by `Debug`, which
/// shows them): bbtree's docs say which of two equal keys a map keeps
/// (`insert`: "the map keeps its key and drops the given one"; `append`:
/// "`self` keeps its key"; set `replace` puts the given one in), as std's
/// map does.
pub(crate) fn eqd<T: PartialEq + Debug + ?Sized>(a: &T, b: &T) -> bool {
    a == b && (!TAGGED.with(|t| t.get()) || format!("{a:?}") == format!("{b:?}"))
}

pub(crate) fn live() -> i64 {
    LIVE.with(|c| c.get())
}
pub(crate) fn pages() -> i64 {
    PAGES.with(|c| c.get())
}

/// Resets the counters and the tags for a new case of key type `K`
/// (other oracles that use these types: sametree).
pub(crate) fn reset<K: UKey>() {
    LIVE.with(|c| c.set(0));
    PAGES.with(|c| c.set(0));
    CLONE_BOMB.with(|c| c.set(0));
    TAG.with(|t| t.set(0));
    TAGGED.with(|t| t.set(K::TAGGED));
}

/// The tag keys made from now on get.
pub(crate) fn set_tag(tag: u32) {
    TAG.with(|t| t.set(tag));
}

/// Counts the pages out; freed pages are scribbled over.
#[derive(Clone, Copy, Debug, Default)]
pub struct Pages;

unsafe impl Allocator for Pages {
    fn allocate_zeroed(&self, layout: Layout) -> Result<NonNull<u8>, AllocError> {
        PAGES.with(|c| c.set(c.get() + 1));
        Global.allocate_zeroed(layout)
    }
    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        PAGES.with(|c| c.set(c.get() - 1));
        unsafe {
            ptr.write_bytes(0xA5, layout.size());
            Global.deallocate(ptr, layout);
        }
    }
}

/// A value that counts how many of it are alive, `P` words of padding to
/// change the leaves' layout, and a clone that can be made to panic.
pub struct Val<const P: usize, A: Align = ()> {
    n: u64,
    pad: [u64; P],
    _align: A,
}

/// A zero-sized field that sets a value's alignment (and so rounds its
/// size up to it), which moves its leaves' entries onto other strides.
pub trait Align: Copy + Default + 'static {}
impl Align for () {}
#[derive(Clone, Copy, Default)]
#[repr(align(16))]
pub struct A16;
impl Align for A16 {}
#[derive(Clone, Copy, Default)]
#[repr(align(64))]
pub struct A64;
impl Align for A64 {}
#[derive(Clone, Copy, Default)]
#[repr(align(128))]
pub struct A128;
impl Align for A128 {}

/// What the model needs of a value: made from a number, bumped, read back
/// (checked whole), counted alive.
pub trait UVal: Clone + Default + Eq + Ord + Debug + 'static {
    fn new(n: u64) -> Self;
    fn bump(&mut self, d: u64);
    fn n(&self) -> u64;
}
impl<const P: usize, A: Align> UVal for Val<P, A> {
    fn new(n: u64) -> Self {
        Val::new(n)
    }
    fn bump(&mut self, d: u64) {
        Val::bump(self, d)
    }
    fn n(&self) -> u64 {
        Val::n(self)
    }
}

impl<const P: usize, A: Align> Val<P, A> {
    fn new(n: u64) -> Self {
        LIVE.with(|c| c.set(c.get() + 1));
        Val { n, pad: [n; P], _align: A::default() }
    }
    fn bump(&mut self, d: u64) {
        self.n = self.n.wrapping_add(d);
        self.pad = [self.n; P];
    }
    /// The number, after checking the padding is whole.
    fn n(&self) -> u64 {
        if self.pad.iter().any(|&w| w != self.n) { u64::MAX - 1 } else { self.n }
    }
}

impl<const P: usize, A: Align> Clone for Val<P, A> {
    fn clone(&self) -> Self {
        CLONE_BOMB.with(|b| match b.get() {
            0 => {}
            1 => {
                b.set(0);
                panic!("umodel: clone bomb");
            }
            left => b.set(left - 1),
        });
        Val::new(self.n())
    }
}
impl<const P: usize, A: Align> Default for Val<P, A> {
    fn default() -> Self {
        Val::new(0)
    }
}
impl<const P: usize, A: Align> Drop for Val<P, A> {
    fn drop(&mut self) {
        LIVE.with(|c| c.set(c.get() - 1));
    }
}
impl<const P: usize, A: Align> PartialEq for Val<P, A> {
    fn eq(&self, other: &Self) -> bool {
        self.n() == other.n()
    }
}
impl<const P: usize, A: Align> Eq for Val<P, A> {}
impl<const P: usize, A: Align> PartialOrd for Val<P, A> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl<const P: usize, A: Align> Ord for Val<P, A> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.n().cmp(&other.n())
    }
}
impl<const P: usize, A: Align> Debug for Val<P, A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.n())
    }
}

/// Keys made from a raw number and the case's key mode: modes give short
/// keys, keys behind a shared prefix, keys of 255+ bytes (saturated hash
/// tags), and a mix whose shared prefix comes and goes.
pub trait UKey: Ord + Clone + Debug + 'static {
    fn of(raw: u128, mode: u8) -> Self;
    /// Whether equal keys can differ (a tag `Ord` ignores).
    const TAGGED: bool = false;
    /// An equal key, made by the current op (a fresh tag).
    fn remake(&self) -> Self {
        self.clone()
    }
}

/// A string key with a tag that `Ord` and `Eq` ignore: equal keys made by
/// different ops collide in the map but stay apart in what it hands back.
/// Its `InlineKey` is the string's (the docs' `UserName` example), so under
/// `WithInlineKey` it has inline keys, separators' keys in leaf heads (D49)
/// and hash tags, which `Auto` (no inline keys for user types) has not.
#[derive(Clone, Debug)]
#[allow(dead_code)] // the tag is read by Debug
pub struct Tagged {
    s: String,
    tag: u32,
}
impl PartialEq for Tagged {
    fn eq(&self, other: &Self) -> bool {
        self.s == other.s
    }
}
impl Eq for Tagged {}
impl PartialOrd for Tagged {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Tagged {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.s.cmp(&other.s)
    }
}
impl bbtree::InlineKey for Tagged {
    const HAS_HASH_TAG: bool = true;
    const HAS_CMP_FROM: bool = true;
    fn inline_key_at(&self, offset: usize) -> bbtree::InlineWord {
        self.s.inline_key_at(offset)
    }
    fn hash_tag(&self) -> u8 {
        self.s.hash_tag()
    }
    fn cmp_from(&self, other: &Self, equal_len: usize) -> std::cmp::Ordering {
        self.s.cmp_from(&other.s, equal_len)
    }
    fn cmp_prefix(&self, prefix: &[u8]) -> std::cmp::Ordering {
        self.s.cmp_prefix(prefix)
    }
}
impl UKey for Tagged {
    fn of(raw: u128, mode: u8) -> Self {
        Tagged { s: String::of(raw, mode), tag: TAG.with(|t| t.get()) }
    }
    const TAGGED: bool = true;
    fn remake(&self) -> Self {
        Tagged { s: self.s.clone(), tag: TAG.with(|t| t.get()) }
    }
}

/// A `u64` key with a tag, as [`Tagged`]; its `InlineKey` is `u64`'s.
#[derive(Clone, Debug)]
#[allow(dead_code)] // the tag is read by Debug
pub struct TagU64 {
    n: u64,
    tag: u32,
}
impl PartialEq for TagU64 {
    fn eq(&self, other: &Self) -> bool {
        self.n == other.n
    }
}
impl Eq for TagU64 {}
impl PartialOrd for TagU64 {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for TagU64 {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.n.cmp(&other.n)
    }
}
impl bbtree::InlineKey for TagU64 {
    const WIDTH: bbtree::InlineWidth = <u64 as bbtree::InlineKey>::WIDTH;
    const HAS_HASH_TAG: bool = <u64 as bbtree::InlineKey>::HAS_HASH_TAG;
    const HAS_CMP_FROM: bool = <u64 as bbtree::InlineKey>::HAS_CMP_FROM;
    fn inline_key_at(&self, offset: usize) -> bbtree::InlineWord {
        self.n.inline_key_at(offset)
    }
    fn hash_tag(&self) -> u8 {
        self.n.hash_tag()
    }
    fn cmp_from(&self, other: &Self, equal_len: usize) -> std::cmp::Ordering {
        self.n.cmp_from(&other.n, equal_len)
    }
    fn cmp_prefix(&self, prefix: &[u8]) -> std::cmp::Ordering {
        self.n.cmp_prefix(prefix)
    }
}
impl UKey for TagU64 {
    fn of(raw: u128, mode: u8) -> Self {
        TagU64 { n: u64::of(raw, mode), tag: TAG.with(|t| t.get()) }
    }
    const TAGGED: bool = true;
    fn remake(&self) -> Self {
        TagU64 { n: self.n, tag: TAG.with(|t| t.get()) }
    }
}

impl UKey for u64 {
    fn of(raw: u128, mode: u8) -> Self {
        match mode % 4 {
            0 => raw as u64,
            1 => (raw as u64).wrapping_add(0xFFFF_FFFF_0000_0000),
            2 => (raw as u64).rotate_right(8),
            _ => (raw as u64) ^ ((raw >> 64) as u64),
        }
    }
}
impl UKey for i128 {
    fn of(raw: u128, mode: u8) -> Self {
        let r = raw as i128;
        match mode % 4 {
            0 => r,
            1 => (r - 30_000).wrapping_mul(1 << 70).wrapping_add(12345),
            2 => r.wrapping_neg(),
            _ => r.rotate_left(64),
        }
    }
}
impl<const N: usize> UKey for [u8; N] {
    fn of(raw: u128, mode: u8) -> Self {
        let be = raw.to_be_bytes();
        let fill = if mode % 2 == 1 && raw % 9 != 0 { 0xAB } else { 0 };
        let mut out = [fill; N];
        for (i, byte) in out.iter_mut().rev().enumerate().take(16) {
            *byte = be[15 - i];
        }
        out
    }
}

pub(crate) fn bytes_of(raw: u128, mode: u8) -> Vec<u8> {
    if crate::zkey::on() {
        return crate::zkey::bytes_of(raw, mode);
    }
    let be = raw.to_be_bytes();
    let skip = be.iter().take_while(|&&b| b == 0).count();
    let digits = &be[skip..];
    let mut k = Vec::new();
    match mode % 4 {
        0 => {}
        1 => k.extend_from_slice(b"tenant/0042/user/"),
        2 => k.resize(300, b'p'),
        _ => match raw % 5 {
            0 => {}
            1 | 2 => k.extend_from_slice(b"tenant/0042/user/"),
            3 => k.resize(260, b'q'),
            _ => k.resize(258, b'p'),
        },
    }
    k.push(digits.len() as u8 + b'a');
    k.extend_from_slice(digits);
    k
}

impl UKey for Vec<u8> {
    fn of(raw: u128, mode: u8) -> Self {
        bytes_of(raw, mode)
    }
}
impl UKey for Box<[u8]> {
    fn of(raw: u128, mode: u8) -> Self {
        bytes_of(raw, mode).into_boxed_slice()
    }
}
// Small and odd key types for the typemx properties: one- and two-byte
// keys (nodes of tiny slots, up to the 1023-key cap), keys of a few values
// (`bool`, `()`: every insert collides), tuples, `char` (gaps at the
// surrogates), and an over-aligned key (64-byte strides).
impl UKey for u8 {
    fn of(raw: u128, mode: u8) -> Self {
        if mode % 2 == 1 { (raw as u8).reverse_bits() } else { raw as u8 }
    }
}
impl UKey for i16 {
    fn of(raw: u128, mode: u8) -> Self {
        match mode % 4 {
            0 => raw as i16,
            1 => (raw as i16).wrapping_sub(i16::MAX / 2),
            2 => (raw as i16).wrapping_neg(),
            _ => (raw as i16).rotate_left(8),
        }
    }
}
impl UKey for u32 {
    fn of(raw: u128, mode: u8) -> Self {
        match mode % 4 {
            0 => raw as u32,
            1 => (raw as u32).wrapping_add(0xFFFF_0000),
            2 => (raw as u32).rotate_right(4),
            _ => (raw as u32) ^ ((raw >> 64) as u32),
        }
    }
}
impl UKey for char {
    fn of(raw: u128, mode: u8) -> Self {
        let c = match mode % 2 {
            0 => raw as u32 % 0x11_0000,
            _ => (0xD7F0 + raw as u32 % 0x40) % 0x11_0000,
        };
        char::from_u32(c).unwrap_or(char::from_u32(0xE000 + c % 0x100).unwrap())
    }
}
impl UKey for bool {
    fn of(raw: u128, mode: u8) -> Self {
        (raw as u8 ^ mode) % 2 == 1
    }
}
impl UKey for () {
    fn of(_: u128, _: u8) -> Self {}
}
impl UKey for (u8, u16) {
    fn of(raw: u128, mode: u8) -> Self {
        let r = raw as u64;
        if mode % 2 == 0 { ((r >> 4) as u8, (r & 0xF) as u16) } else { ((r % 3) as u8, r as u16) }
    }
}
impl UKey for Option<u64> {
    fn of(raw: u128, mode: u8) -> Self {
        if raw % 7 == 0 && mode % 2 == 0 { None } else { Some(u64::of(raw, mode)) }
    }
}
/// A key aligned to 64 bytes, compared by its number.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(align(64))]
pub struct K64(u32);
impl UKey for K64 {
    fn of(raw: u128, mode: u8) -> Self {
        K64(u32::of(raw, mode))
    }
}
impl UKey for String {
    fn of(raw: u128, mode: u8) -> Self {
        if crate::zkey::on() {
            return String::from_utf8(crate::zkey::bytes_of(raw, mode)).expect("zkey keys are ASCII");
        }
        bytes_of(raw, mode).iter().map(|&b| if b.is_ascii_graphic() { b as char } else { char::from(b'0' + b % 10) }).collect()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Op {
    Insert { b: bool, k: u128, v: u64 },
    /// `remove`, or `remove_entry` if `entry`.
    Remove { b: bool, k: u128, entry: bool },
    GetMut { b: bool, k: u128, d: u64 },
    /// One of the entry API's paths, by `act`.
    Entry { b: bool, k: u128, act: u8, v: u64 },
    /// `first_entry` or `last_entry`, then one of its methods.
    End { b: bool, last: bool, act: u8, v: u64 },
    /// A walk by `kind`: iter, keys, values, range, iter_mut, values_mut,
    /// range_mut. Mutable walks add `d` to each value they visit.
    Walk { b: bool, kind: u8, lo: RawBound, hi: RawBound, walk: Vec<bool>, d: u64 },
    /// into_iter / into_keys / into_values, `walk` steps, then the rest is
    /// dropped (or drained, if `drain`); the map is empty after.
    Consume { b: bool, kind: u8, walk: Vec<bool>, drain: bool },
    /// Clones; the clone replaces the map if `keep`. With `bomb` > 0 the
    /// `bomb`-th value clone panics (if there are that many values).
    Clone { b: bool, bomb: u32, keep: bool },
    /// The split-off part replaces the other map.
    SplitOff { b: bool, k: u128 },
    Append { b: bool },
    Retain { b: bool, m: u8, r: u8 },
    ExtractIf { b: bool, lo: RawBound, hi: RawBound, m: u8, r: u8, take: u32 },
    Pop { b: bool, last: bool, n: u32 },
    Clear { b: bool },
    Extend { b: bool, start: u128, stride: u8, len: u16 },
    /// `==`, `cmp`, `Index`, `get_key_value`, `contains_key`.
    Compare { k: u128 },
    Many { b: bool, ks: [u128; 4], v: u64, insert: bool },
    Swap,
    /// Set: insert, remove, replace, take, get/contains, by `act`.
    SOne { b: bool, k: u128, act: u8 },
    /// The set becomes the keys of the map on its side whose value is not
    /// ≡ `r` mod `m`.
    SFromMap { b: bool, m: u8, r: u8 },
    /// difference / intersection / union / symmetric_difference, the
    /// first `take` items; with `build`, the operator that builds a set.
    SAlgebra { kind: u8, swap: bool, take: u32, build: bool },
    SRel,
    SWalk { b: bool, lo: RawBound, hi: RawBound, walk: Vec<bool>, range: bool },
    /// split_off / append / retain / pop / extract_if / into_iter / clone /
    /// clear, by `act`.
    SBulk { b: bool, act: u8, k: u128, m: u8, r: u8, n: u32 },
    SMany { b: bool, ks: [u128; 4], insert: bool },
    /// The iterator methods the iterators implement on their own: `last`,
    /// `min`, `max`, `nth`, `nth_back`, `count`, on the maps' and sets'
    /// walks and set algebra, and an iterator cloned part way.
    Extremes { b: bool, lo: RawBound, hi: RawBound, j: u16 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Case {
    pub mode: u8,
    pub a: MapSeed,
    pub b: MapSeed,
    pub ops: Vec<Op>,
}

fn walk() -> BoxGen<Vec<bool>> {
    gens::vec(gens::bool()).len(0..=40).boxed()
}

fn op() -> BoxGen<Op> {
    let k = raw_key();
    let v = gens::int(0u64..=u64::MAX);
    let side = gens::bool();
    let bound = RawBound::generator();
    let small = gens::int(0u8..=255);
    let count = gens::frequency(vec![(3, gens::int(0u32..=16).boxed()), (2, gens::int(0u32..=2_000).boxed()), (1, gens::int(0u32..=50_000).boxed())]);
    let ks = (k.clone(), k.clone(), k.clone(), k.clone()).map(|(a, b, c, d)| [a, b, c, d]).boxed();
    gens::frequency(vec![
        (6, (side.clone(), k.clone(), v.clone()).map(|(b, k, v)| Op::Insert { b, k, v }).boxed()),
        (4, (side.clone(), k.clone(), gens::bool()).map(|(b, k, entry)| Op::Remove { b, k, entry }).boxed()),
        (2, (side.clone(), k.clone(), v.clone()).map(|(b, k, d)| Op::GetMut { b, k, d }).boxed()),
        (8, (side.clone(), k.clone(), small.clone(), v.clone()).map(|(b, k, act, v)| Op::Entry { b, k, act, v }).boxed()),
        (3, (side.clone(), gens::bool(), small.clone(), v.clone()).map(|(b, last, act, v)| Op::End { b, last, act, v }).boxed()),
        (
            6,
            (side.clone(), gens::int(0u8..=6), bound.clone(), bound.clone(), walk(), v.clone())
                .map(|(b, kind, lo, hi, walk, d)| Op::Walk { b, kind, lo, hi, walk, d })
                .boxed(),
        ),
        (1, (side.clone(), gens::int(0u8..=2), walk(), gens::bool()).map(|(b, kind, walk, drain)| Op::Consume { b, kind, walk, drain }).boxed()),
        (
            2,
            (side.clone(), gens::frequency(vec![(2, gens::constant(0u32).boxed()), (1, gens::int(1u32..=3_000).boxed())]), gens::bool())
                .map(|(b, bomb, keep)| Op::Clone { b, bomb, keep })
                .boxed(),
        ),
        (2, (side.clone(), k.clone()).map(|(b, k)| Op::SplitOff { b, k }).boxed()),
        (1, side.clone().map(|b| Op::Append { b }).boxed()),
        (1, (side.clone(), gens::int(1u8..=7), small.clone()).map(|(b, m, r)| Op::Retain { b, m, r: r % m }).boxed()),
        (
            1,
            (side.clone(), bound.clone(), bound.clone(), gens::int(1u8..=5), small.clone(), count.clone())
                .map(|(b, lo, hi, m, r, take)| Op::ExtractIf { b, lo, hi, m, r: r % m, take })
                .boxed(),
        ),
        (1, (side.clone(), gens::bool(), count.clone()).map(|(b, last, n)| Op::Pop { b, last, n }).boxed()),
        (1, side.clone().map(|b| Op::Clear { b }).boxed()),
        (
            1,
            (side.clone(), k.clone(), gens::int(1u8..=9), gens::int(0u16..=5_000))
                .map(|(b, start, stride, len)| Op::Extend { b, start, stride, len })
                .boxed(),
        ),
        (2, k.clone().map(|k| Op::Compare { k }).boxed()),
        (2, (side.clone(), ks.clone(), v.clone(), gens::bool()).map(|(b, ks, v, insert)| Op::Many { b, ks, v, insert }).boxed()),
        (1, gens::constant(Op::Swap).boxed()),
        (5, (side.clone(), k.clone(), small.clone()).map(|(b, k, act)| Op::SOne { b, k, act }).boxed()),
        (2, (side.clone(), gens::int(1u8..=7), small.clone()).map(|(b, m, r)| Op::SFromMap { b, m, r: r % m }).boxed()),
        (
            4,
            (gens::int(0u8..=3), gens::bool(), count.clone(), gens::bool())
                .map(|(kind, swap, take, build)| Op::SAlgebra { kind, swap, take, build })
                .boxed(),
        ),
        (2, gens::constant(Op::SRel).boxed()),
        (
            2,
            (side.clone(), bound.clone(), bound, walk(), gens::bool()).map(|(b, lo, hi, walk, range)| Op::SWalk { b, lo, hi, walk, range }).boxed(),
        ),
        (
            3,
            (side.clone(), gens::int(0u8..=7), k.clone(), gens::int(1u8..=7), small, count)
                .map(|(b, act, k, m, r, n)| Op::SBulk { b, act, k, m, r: r % m, n })
                .boxed(),
        ),
        (2, (side.clone(), ks, gens::bool()).map(|(b, ks, insert)| Op::SMany { b, ks, insert }).boxed()),
        (
            3,
            (side, RawBound::generator(), RawBound::generator(), gens::frequency(vec![(3, gens::int(0u16..=40).boxed()), (1, gens::int(0u16..=u16::MAX).boxed())]))
                .map(|(b, lo, hi, j)| Op::Extremes { b, lo, hi, j })
                .boxed(),
        ),
    ])
    .boxed()
}

pub fn case() -> BoxGen<Case> {
    (gens::int(0u8..=3), MapSeed::generator(), MapSeed::generator(), gens::vec(op()).len(1..=40))
        .iso(
            |(mode, a, b, ops)| Case { mode, a, b, ops },
            |c: &Case| (c.mode, c.a.clone(), c.b.clone(), c.ops.clone()),
        )
        .boxed()
}

/// The bounds, made valid: an inverted range panics in std and in bbtree
/// alike, which the syncmap property checks; here the walk matters.
fn bounds<K: UKey>(lo: RawBound, hi: RawBound, mode: u8) -> (Bound<K>, Bound<K>) {
    let to = |b: RawBound| match b {
        RawBound::Included(r) => Bound::Included(K::of(r, mode)),
        RawBound::Excluded(r) => Bound::Excluded(K::of(r, mode)),
        RawBound::Unbounded => Bound::Unbounded,
    };
    let (lo, hi) = (to(lo), to(hi));
    if range_panics(&lo, &hi) { (lo, Bound::Unbounded) } else { (lo, hi) }
}

/// Follows `walk` over `ours`, against `want` (what it must give, in
/// order), checking `size_hint` at every step (exactly, if `exact`), then
/// drains the rest, and checks the iterator stays finished.
fn walk_check<T: PartialEq + Debug>(mut ours: impl DoubleEndedIterator<Item = T>, want: Vec<T>, walk: &[bool], exact: bool) -> Result<(), (&'static str, String)> {
    let (mut front, mut back) = (0usize, want.len());
    let hint = |ours: &dyn Fn() -> (usize, Option<usize>), left: usize, step: usize| -> Result<(), (&'static str, String)> {
        let (lo, hi) = ours();
        let ok = lo <= left && hi.is_none_or(|h| left <= h) && (!exact || (lo == left && hi == Some(left)));
        if ok { Ok(()) } else { Err(("size_hint", format!("step {step}: size_hint {:?} with {left} left", (lo, hi)))) }
    };
    for (step, &f) in walk.iter().enumerate() {
        hint(&|| ours.size_hint(), back - front, step)?;
        let got = if f { ours.next() } else { ours.next_back() };
        let expected = if front == back {
            None
        } else if f {
            front += 1;
            Some(&want[front - 1])
        } else {
            back -= 1;
            Some(&want[back])
        };
        if !eqd(&got.as_ref(), &expected) {
            return Err(("walk", format!("step {step} ({}): ours {got:?}, std {expected:?}", if f { "next" } else { "next_back" })));
        }
    }
    // The rest: by `next`, or (by the walk's length) by `fold`, `rfold`
    // or `count`, which the iterators implement on their own.
    match walk.len() % 4 {
        1 => {
            let rest = ours.fold(Vec::new(), |mut acc, x| {
                acc.push(x);
                acc
            });
            return same_rest("fold", rest.iter(), &want[front..back]);
        }
        2 => {
            let mut rest = ours.rfold(Vec::new(), |mut acc, x| {
                acc.push(x);
                acc
            });
            rest.reverse();
            return same_rest("rfold", rest.iter(), &want[front..back]);
        }
        3 => {
            let n = ours.count();
            return if n == back - front { Ok(()) } else { Err(("walk", format!("count gives {n}, {} left", back - front))) };
        }
        _ => {}
    }
    let mut step = walk.len();
    while front < back {
        hint(&|| ours.size_hint(), back - front, step)?;
        let got = ours.next();
        if !eqd(&got.as_ref(), &Some(&want[front])) {
            return Err(("walk", format!("draining at {front} of {}: ours {got:?}, std {:?}", want.len(), want[front])));
        }
        front += 1;
        step += 1;
    }
    hint(&|| ours.size_hint(), 0, step)?;
    if let Some(x) = ours.next() {
        return Err(("walk", format!("ours gives {x:?} after the end")));
    }
    if let Some(x) = ours.next_back() {
        return Err(("walk", format!("ours gives {x:?} from the back after the end")));
    }
    Ok(())
}

fn same_rest<'a, T: PartialEq + Debug + 'a>(how: &str, rest: impl Iterator<Item = &'a T>, want: &[T]) -> Result<(), (&'static str, String)> {
    let rest: Vec<&T> = rest.collect();
    if rest.len() == want.len() && rest.iter().zip(want).all(|(a, b)| eqd(*a, b)) {
        return Ok(());
    }
    let at = rest.iter().zip(want).position(|(a, b)| !eqd(*a, b));
    Err(("walk", format!("the rest by {how}: {} items, want {}; first difference at {at:?}: {:?} vs {:?}", rest.len(), want.len(), at.map(|i| rest[i]), at.map(|i| &want[i]))))
}

/// The same for forward-only iterators (set algebra), `take` items; with
/// `take` ≢ 0 mod 3, `take % 7` items and then the rest by `fold` or
/// `count`.
fn walk_forward<T: PartialEq + Debug>(mut ours: impl Iterator<Item = T>, want: &[T], take: usize) -> Result<(), (&'static str, String)> {
    let mode = take % 3;
    let take = if mode == 0 { take } else { take % 7 };
    for i in 0..take.min(want.len()) {
        let (lo, hi) = ours.size_hint();
        let left = want.len() - i;
        if lo > left || hi.is_some_and(|h| h < left) {
            return Err(("size_hint", format!("item {i}: size_hint {:?} with {left} left", (lo, hi))));
        }
        let got = ours.next();
        if !eqd(&got.as_ref(), &Some(&want[i])) {
            return Err(("walk", format!("item {i} of {}: ours {got:?}, std {:?}", want.len(), want[i])));
        }
    }
    let done = take.min(want.len());
    match mode {
        1 => {
            let rest = ours.fold(Vec::new(), |mut acc, x| {
                acc.push(x);
                acc
            });
            return same_rest("fold", rest.iter(), &want[done..]);
        }
        2 => {
            let n = ours.count();
            return if n == want.len() - done { Ok(()) } else { Err(("walk", format!("count gives {n}, {} left", want.len() - done))) };
        }
        _ => {}
    }
    if take >= want.len() {
        let (lo, _) = ours.size_hint();
        if lo > 0 {
            return Err(("size_hint", format!("at the end: size_hint lower {lo}")));
        }
        if let Some(x) = ours.next() {
            return Err(("walk", format!("ours gives {x:?} after the end ({} items)", want.len())));
        }
    }
    Ok(())
}

#[allow(unused_variables)]
fn invariants_map<K: UKey, V, S: InlineKeyStrategy<K>>(map: &BBTreeMap<K, V, S, Pages>, at: &str) -> Result<(), Fail> {
    #[cfg(fuzzing)]
    {
        let r = catch_unwind(AssertUnwindSafe(|| {
            map.check_invariants();
            map.check_prefix_lens();
        }));
        if let Err(p) = r {
            let msg = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()));
            return Err(fail("umodel__invariant__map", format!("{at}: {msg:?}")));
        }
    }
    Ok(())
}

#[allow(unused_variables)]
fn invariants_set<K: UKey, S: InlineKeyStrategy<K>>(set: &BBTreeSet<K, S, Pages>, at: &str) -> Result<(), Fail> {
    #[cfg(fuzzing)]
    {
        let r = catch_unwind(AssertUnwindSafe(|| set.check_invariants()));
        if let Err(p) = r {
            let msg = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()));
            return Err(fail("umodel__invariant__set", format!("{at}: {msg:?}")));
        }
    }
    Ok(())
}

type Map<K, V, S> = BBTreeMap<K, V, S, Pages>;
type Set<K, S> = BBTreeSet<K, S, Pages>;

fn same_map<K: UKey, V: UVal, S: InlineKeyStrategy<K>>(map: &Map<K, V, S>, model: &BTreeMap<K, u64>) -> Option<String> {
    if map.len() != model.len() {
        return Some(format!("len {} vs {}", map.len(), model.len()));
    }
    let pos = map.iter().map(|(k, v)| (k, v.n())).zip(model.iter().map(|(k, v)| (k, *v))).position(|(a, b)| !eqd(&a, &b))?;
    let ours = map.iter().nth(pos).map(|(k, v)| (k.clone(), v.n()));
    Some(format!("entry {pos} of {}: ours {ours:?}, std {:?}", model.len(), model.iter().nth(pos)))
}

fn same_set<K: UKey, S: InlineKeyStrategy<K>>(set: &Set<K, S>, model: &BTreeSet<K>) -> Option<String> {
    if set.len() != model.len() {
        return Some(format!("len {} vs {}", set.len(), model.len()));
    }
    let pos = set.iter().zip(model.iter()).position(|(a, b)| !eqd(&a, &b))?;
    Some(format!("element {pos} of {}: ours {:?}, std {:?}", model.len(), set.iter().nth(pos), model.iter().nth(pos)))
}

/// Two of each, A and B, by `b`.
fn pick<T>(pair: &mut (T, T), b: bool) -> &mut T {
    if b { &mut pair.1 } else { &mut pair.0 }
}

fn new_map<K, V: UVal, S: InlineKeyStrategy<K> + Default>() -> Map<K, V, S> {
    BBTreeMap::with_strategy_in(S::default(), Pages)
}
fn new_set<K, S: InlineKeyStrategy<K> + Default>() -> Set<K, S> {
    BBTreeSet::with_strategy_in(S::default(), Pages)
}

pub fn run<K: UKey, V: UVal, S: InlineKeyStrategy<K> + Default>(case: &Case) -> Result<(), Fail> {
    LIVE.with(|c| c.set(0));
    PAGES.with(|c| c.set(0));
    CLONE_BOMB.with(|c| c.set(0));
    TAG.with(|t| t.set(0));
    TAGGED.with(|t| t.set(K::TAGGED));
    let r = run_inner::<K, V, S>(case);
    CLONE_BOMB.with(|c| c.set(0));
    r?;
    ensure(live() == 0, "umodel__leak__values", || format!("{} values alive after every collection dropped", live()))?;
    ensure(pages() == 0, "umodel__leak__pages", || format!("{} pages out after every collection dropped", pages()))
}

fn run_inner<K: UKey, V: UVal, S: InlineKeyStrategy<K> + Default>(case: &Case) -> Result<(), Fail> {
    let mode = case.mode;
    let key = |r: u128| K::of(r, mode);
    let mut maps: (Map<K, V, S>, Map<K, V, S>) = (new_map(), new_map());
    let mut models: (BTreeMap<K, u64>, BTreeMap<K, u64>) = (BTreeMap::new(), BTreeMap::new());
    let mut sets: (Set<K, S>, Set<K, S>) = (new_set(), new_set());
    let mut smodels: (BTreeSet<K>, BTreeSet<K>) = (BTreeSet::new(), BTreeSet::new());
    for (side, seed) in [(false, &case.a), (true, &case.b)] {
        for raw in seed.raw_keys() {
            let (k, v) = (key(raw), seed.value_of(raw));
            pick(&mut maps, side).insert(k.clone(), V::new(v));
            pick(&mut models, side).insert(k.clone(), v);
            if raw % 3 != 0 {
                pick(&mut sets, side).insert(k.clone());
                pick(&mut smodels, side).insert(k);
            }
        }
    }
    let (mut algebra_hits, mut bomb_fired, mut entries) = (false, false, false);
    for (i, op) in case.ops.iter().enumerate() {
        TAG.with(|t| t.set(i as u32 + 1));
        let at = |why: String| format!("op {i} {op:?}: {why}");
        let wf = |area: &str, (kind, msg): (&str, String)| fail(&format!("umodel__{kind}__{area}"), at(msg));
        match op {
            Op::Insert { b, k, v } => {
                let (x, y) = (pick(&mut maps, *b).insert(key(*k), V::new(*v)).map(|v| v.n()), pick(&mut models, *b).insert(key(*k), *v));
                ensure(eqd(&x, &y), "umodel__mismatch__insert", || at(format!("{x:?} vs {y:?}")))?;
            }
            Op::Remove { b, k, entry } => {
                let k = key(*k);
                if *entry {
                    let x = pick(&mut maps, *b).remove_entry(&k).map(|(k, v)| (k, v.n()));
                    let y = pick(&mut models, *b).remove_entry(&k);
                    ensure(eqd(&x, &y), "umodel__mismatch__remove_entry", || at(format!("{x:?} vs {y:?}")))?;
                } else {
                    let (x, y) = (pick(&mut maps, *b).remove(&k).map(|v| v.n()), pick(&mut models, *b).remove(&k));
                    ensure(eqd(&x, &y), "umodel__mismatch__remove", || at(format!("{x:?} vs {y:?}")))?;
                }
            }
            Op::GetMut { b, k, d } => {
                let k = key(*k);
                let x = pick(&mut maps, *b).get_mut(&k).map(|v| {
                    v.bump(*d);
                    v.n()
                });
                let y = pick(&mut models, *b).get_mut(&k).map(|v| {
                    *v = v.wrapping_add(*d);
                    *v
                });
                ensure(eqd(&x, &y), "umodel__mismatch__get_mut", || at(format!("{x:?} vs {y:?}")))?;
            }
            Op::Entry { b, k, act, v } => {
                entries = true;
                let (map, model) = (pick(&mut maps, *b), pick(&mut models, *b));
                let k = key(*k);
                let (x, y) = entry_op(map, model, k, *act, *v);
                ensure(eqd(&x, &y), "umodel__mismatch__entry", || at(format!("{x:?} vs {y:?}")))?;
            }
            Op::End { b, last, act, v } => {
                let (map, model) = (pick(&mut maps, *b), pick(&mut models, *b));
                let (x, y) = end_op(map, model, *last, *act, *v);
                ensure(eqd(&x, &y), "umodel__mismatch__first_last_entry", || at(format!("{x:?} vs {y:?}")))?;
            }
            Op::Walk { b, kind, lo, hi, walk, d } => {
                let (map, model) = (pick(&mut maps, *b), pick(&mut models, *b));
                let (lo, hi) = bounds::<K>(*lo, *hi, mode);
                let r = match kind {
                    0 => walk_check(map.iter().map(|(k, v)| (k.clone(), v.n())), model.iter().map(|(k, v)| (k.clone(), *v)).collect(), walk, true)
                        .map_err(|e| wf("iter", e)),
                    1 => walk_check(map.keys().cloned(), model.keys().cloned().collect(), walk, true).map_err(|e| wf("keys", e)),
                    2 => walk_check(map.values().map(|v| v.n()), model.values().copied().collect(), walk, true).map_err(|e| wf("values", e)),
                    3 => walk_check(
                        map.range((lo.clone(), hi.clone())).map(|(k, v)| (k.clone(), v.n())),
                        model.range((lo, hi)).map(|(k, v)| (k.clone(), *v)).collect(),
                        walk,
                        false,
                    )
                    .map_err(|e| wf("range", e)),
                    4 => {
                        let want = model.iter_mut().map(|(k, v)| {
                            let old = *v;
                            *v = v.wrapping_add(*d);
                            (k.clone(), old)
                        });
                        let want = want.collect();
                        let ours = map.iter_mut().map(|(k, v)| {
                            let old = v.n();
                            v.bump(*d);
                            (k.clone(), old)
                        });
                        walk_check(ours, want, walk, true).map_err(|e| wf("iter_mut", e))
                    }
                    5 => {
                        let want = model.values_mut().map(|v| std::mem::replace(v, v.wrapping_add(*d))).collect();
                        let ours = map.values_mut().map(|v| {
                            let old = v.n();
                            v.bump(*d);
                            old
                        });
                        walk_check(ours, want, walk, true).map_err(|e| wf("values_mut", e))
                    }
                    _ => {
                        let want = model
                            .range_mut((lo.clone(), hi.clone()))
                            .map(|(k, v)| (k.clone(), std::mem::replace(v, v.wrapping_add(*d))))
                            .collect();
                        let ours = map.range_mut((lo, hi)).map(|(k, v)| {
                            let old = v.n();
                            v.bump(*d);
                            (k.clone(), old)
                        });
                        walk_check(ours, want, walk, false).map_err(|e| wf("range_mut", e))
                    }
                };
                r?;
            }
            Op::Consume { b, kind, walk, drain } => {
                let map = std::mem::replace(pick(&mut maps, *b), new_map());
                let model = std::mem::take(pick(&mut models, *b));
                let n = if *drain { usize::MAX } else { walk.len() };
                let r = match kind {
                    0 => walk_check(
                        map.into_iter().map(|(k, v)| (k, v.n())).take(n),
                        model.into_iter().take(n).collect(),
                        &[],
                        false,
                    )
                    .map_err(|e| wf("into_iter", e)),
                    1 => walk_check(map.into_keys().take(n), model.into_keys().take(n).collect(), &[], false).map_err(|e| wf("into_keys", e)),
                    _ => walk_check(map.into_values().map(|v| v.n()).take(n), model.into_values().take(n).collect(), &[], false)
                        .map_err(|e| wf("into_values", e)),
                };
                r?;
            }
            Op::Clone { b, bomb, keep } => {
                let (map, model) = (pick(&mut maps, *b), pick(&mut models, *b));
                let (live0, pages0) = (live(), pages());
                CLONE_BOMB.with(|c| c.set(u64::from(*bomb)));
                let copy = catch_unwind(AssertUnwindSafe(|| map.clone()));
                CLONE_BOMB.with(|c| c.set(0));
                match copy {
                    Err(_) => {
                        bomb_fired = true;
                        ensure(live() == live0, "umodel__leak__clone_panic", || at(format!("{} values alive, {live0} before the clone", live())))?;
                        ensure(pages() == pages0, "umodel__leak__clone_panic", || at(format!("{} pages out, {pages0} before the clone", pages())))?;
                        if let Some(why) = same_map(map, model) {
                            return Err(fail("umodel__mismatch__clone_panic", at(format!("the original: {why}"))));
                        }
                    }
                    Ok(copy) => {
                        if let Some(why) = same_map(&copy, model) {
                            return Err(fail("umodel__mismatch__clone", at(why)));
                        }
                        ensure(copy == *map, "umodel__mismatch__clone", || at("clone != original".into()))?;
                        invariants_map(&copy, &at("the clone".into()))?;
                        if *keep {
                            *map = copy;
                        }
                    }
                }
            }
            Op::SplitOff { b, k } => {
                let k = key(*k);
                let x = pick(&mut maps, *b).split_off(&k);
                let y = pick(&mut models, *b).split_off(&k);
                *pick(&mut maps, !*b) = x;
                *pick(&mut models, !*b) = y;
            }
            Op::Append { b } => {
                let (x, y) = if *b { (&mut maps.1, &mut maps.0) } else { (&mut maps.0, &mut maps.1) };
                x.append(y);
                let (x, y) = if *b { (&mut models.1, &mut models.0) } else { (&mut models.0, &mut models.1) };
                x.append(y);
            }
            Op::Retain { b, m, r } => {
                let keep = |v: u64| (v % u64::from(*m)) as u8 != *r;
                let mut seen = 0usize;
                pick(&mut maps, *b).retain(|_, v| {
                    seen += 1;
                    let k = keep(v.n());
                    v.bump(1);
                    k
                });
                pick(&mut models, *b).retain(|_, v| {
                    let k = keep(*v);
                    *v = v.wrapping_add(1);
                    k
                });
                let want = pick(&mut models, *b).len();
                ensure(seen >= want, "umodel__mismatch__retain", || at(format!("the predicate saw {seen} entries")))?;
            }
            Op::ExtractIf { b, lo, hi, m, r, take } => {
                let (lo, hi) = bounds::<K>(*lo, *hi, mode);
                let pick_it = |v: u64| (v % u64::from(*m)) as u8 == *r;
                let x: Vec<_> =
                    pick(&mut maps, *b).extract_if((lo.clone(), hi.clone()), |_, v| pick_it(v.n())).take(*take as usize).map(|(k, v)| (k, v.n())).collect();
                let y: Vec<_> = pick(&mut models, *b).extract_if((lo, hi), |_, v| pick_it(*v)).take(*take as usize).collect();
                ensure(eqd(&x, &y), "umodel__mismatch__extract_if", || at(format!("extracted {} vs {}", x.len(), y.len())))?;
            }
            Op::Pop { b, last, n } => {
                for j in 0..*n {
                    let (x, y) = if *last {
                        (pick(&mut maps, *b).pop_last().map(|(k, v)| (k, v.n())), pick(&mut models, *b).pop_last())
                    } else {
                        (pick(&mut maps, *b).pop_first().map(|(k, v)| (k, v.n())), pick(&mut models, *b).pop_first())
                    };
                    ensure(eqd(&x, &y), "umodel__mismatch__pop", || at(format!("pop {j}: {x:?} vs {y:?}")))?;
                    if y.is_none() {
                        break;
                    }
                }
            }
            Op::Clear { b } => {
                pick(&mut maps, *b).clear();
                pick(&mut models, *b).clear();
            }
            Op::Extend { b, start, stride, len } => {
                let raws: Vec<u128> = (0..u128::from(*len)).map(|j| start.wrapping_add(j * u128::from(*stride))).collect();
                pick(&mut maps, *b).extend(raws.iter().map(|&r| (key(r), V::new(r as u64))));
                pick(&mut models, *b).extend(raws.iter().map(|&r| (key(r), r as u64)));
            }
            Op::Compare { k } => {
                let eq = (maps.0 == maps.1, models.0 == models.1);
                ensure(eq.0 == eq.1, "umodel__mismatch__eq", || at(format!("{eq:?}")))?;
                let ours = maps.0.iter().map(|(k, v)| (k, v.n())).cmp(maps.1.iter().map(|(k, v)| (k, v.n())));
                let cmp = (maps.0.cmp(&maps.1), ours, models.0.cmp(&models.1));
                ensure(cmp.0 == cmp.2 && cmp.1 == cmp.2, "umodel__mismatch__cmp", || at(format!("{cmp:?}")))?;
                let raw = *k as u64;
                let k = key(*k);
                for (map, model) in [(&maps.0, &models.0), (&maps.1, &models.1)] {
                    let x = map.get_key_value(&k).map(|(k, v)| (k.clone(), v.n()));
                    let y = model.get_key_value(&k).map(|(k, v)| (k.clone(), *v));
                    ensure(eqd(&x, &y) && map.contains_key(&k) == y.is_some(), "umodel__mismatch__get", || at(format!("{x:?} vs {y:?}")))?;
                    if let Some((_, v)) = y {
                        ensure(map[&k].n() == v, "umodel__mismatch__index", || at(format!("{:?} vs {v}", map[&k])))?;
                    }
                }
                glue(&mut maps, &models, &sets, &smodels, raw).map_err(|(area, msg)| fail(&format!("umodel__mismatch__{area}"), at(msg)))?;
            }
            Op::Many { b, ks, v, insert } => {
                let (map, model) = (pick(&mut maps, *b), pick(&mut models, *b));
                let ks = ks.map(&key);
                if *insert {
                    let x = map.insert_many(std::array::from_fn::<_, 4, _>(|j| (ks[j].clone(), V::new(v.wrapping_add(j as u64))))).map(|o| o.map(|v| v.n()));
                    let y: [Option<u64>; 4] = std::array::from_fn(|j| model.insert(ks[j].clone(), v.wrapping_add(j as u64)));
                    ensure(eqd(&x, &y), "umodel__mismatch__insert_many", || at(format!("{x:?} vs {y:?}")))?;
                } else {
                    let x = map.get_many([&ks[0], &ks[1], &ks[2], &ks[3]]).map(|o| o.map(|v| v.n()));
                    let y: [Option<u64>; 4] = std::array::from_fn(|j| model.get(&ks[j]).copied());
                    ensure(eqd(&x, &y), "umodel__mismatch__get_many", || at(format!("{x:?} vs {y:?}")))?;
                }
            }
            Op::Swap => {
                std::mem::swap(&mut maps.0, &mut maps.1);
                std::mem::swap(&mut models.0, &mut models.1);
                std::mem::swap(&mut sets.0, &mut sets.1);
                std::mem::swap(&mut smodels.0, &mut smodels.1);
            }
            Op::SOne { b, k, act } => {
                let (set, model) = (pick(&mut sets, *b), pick(&mut smodels, *b));
                let k = key(*k);
                let (x, y) = match act % 6 {
                    0 | 1 => (format!("{:?}", set.insert(k.clone())), format!("{:?}", model.insert(k))),
                    2 => (format!("{:?}", set.remove(&k)), format!("{:?}", model.remove(&k))),
                    // every (act / 6 % 4 + 1)-th element replaced by an equal one: separators too
                    3 if *act >= 128 => {
                        let step = usize::from(*act / 6 % 4) + 1;
                        let olds: Vec<K> = model.iter().step_by(step).cloned().collect();
                        let (mut x, mut y) = (String::new(), String::new());
                        for old in olds {
                            x += &format!("{:?}", set.replace(old.remake()));
                            y += &format!("{:?}", model.replace(old.remake()));
                        }
                        (x, y)
                    }
                    3 => (format!("{:?}", set.replace(k.clone())), format!("{:?}", model.replace(k))),
                    4 => (format!("{:?}", set.take(&k)), format!("{:?}", model.take(&k))),
                    _ => (format!("{:?} {}", set.get(&k), set.contains(&k)), format!("{:?} {}", model.get(&k), model.contains(&k))),
                };
                ensure(eqd(&x, &y), "umodel__mismatch__set_one", || at(format!("{x} vs {y}")))?;
            }
            Op::SFromMap { b, m, r } => {
                let keep = |v: u64| (v % u64::from(*m)) as u8 != *r;
                let keys: Vec<K> = pick(&mut models, *b).iter().filter(|(_, v)| keep(**v)).map(|(k, _)| k.clone()).collect();
                let mut set = new_set();
                set.extend(keys.iter().cloned());
                *pick(&mut sets, *b) = set;
                *pick(&mut smodels, *b) = keys.into_iter().collect();
            }
            Op::SAlgebra { kind, swap, take, build } => {
                let (x, y) = if *swap { (&sets.1, &sets.0) } else { (&sets.0, &sets.1) };
                let (mx, my) = if *swap { (&smodels.1, &smodels.0) } else { (&smodels.0, &smodels.1) };
                let (area, want): (&str, Vec<K>) = match kind {
                    0 => ("difference", mx.difference(my).cloned().collect()),
                    1 => ("intersection", mx.intersection(my).cloned().collect()),
                    2 => ("union", mx.union(my).cloned().collect()),
                    _ => ("symmetric_difference", mx.symmetric_difference(my).cloned().collect()),
                };
                algebra_hits |= !want.is_empty() && !mx.is_empty() && !my.is_empty();
                if *build {
                    let built = match kind {
                        0 => x - y,
                        1 => x & y,
                        2 => x | y,
                        _ => x ^ y,
                    };
                    let model: BTreeSet<K> = want.into_iter().collect();
                    if let Some(why) = same_set(&built, &model) {
                        return Err(fail(&format!("umodel__mismatch__set_op_{area}"), at(why)));
                    }
                    invariants_set(&built, &at(format!("the {area} built")))?;
                } else {
                    let take = *take as usize;
                    let r = match kind {
                        0 => walk_forward(x.difference(y).cloned(), &want, take),
                        1 => walk_forward(x.intersection(y).cloned(), &want, take),
                        2 => walk_forward(x.union(y).cloned(), &want, take),
                        _ => walk_forward(x.symmetric_difference(y).cloned(), &want, take),
                    };
                    r.map_err(|e| wf(area, e))?;
                }
            }
            Op::SRel => {
                let x = [sets.0.is_subset(&sets.1), sets.1.is_subset(&sets.0), sets.0.is_superset(&sets.1), sets.0.is_disjoint(&sets.1)];
                let y = [smodels.0.is_subset(&smodels.1), smodels.1.is_subset(&smodels.0), smodels.0.is_superset(&smodels.1), smodels.0.is_disjoint(&smodels.1)];
                ensure(eqd(&x, &y), "umodel__mismatch__set_relation", || at(format!("subset/subset_rev/superset/disjoint {x:?} vs {y:?}")))?;
                let c = (sets.0 == sets.1, sets.0.cmp(&sets.1), smodels.0 == smodels.1, smodels.0.cmp(&smodels.1));
                ensure(c.0 == c.2 && c.1 == c.3, "umodel__mismatch__set_cmp", || at(format!("{c:?}")))?;
            }
            Op::SWalk { b, lo, hi, walk, range } => {
                let (set, model) = (pick(&mut sets, *b), pick(&mut smodels, *b));
                let r = if *range {
                    let (lo, hi) = bounds::<K>(*lo, *hi, mode);
                    walk_check(set.range((lo.clone(), hi.clone())).cloned(), model.range((lo, hi)).cloned().collect(), walk, false).map_err(|e| wf("set_range", e))
                } else {
                    walk_check(set.iter().cloned(), model.iter().cloned().collect(), walk, true).map_err(|e| wf("set_iter", e))
                };
                r?;
            }
            Op::SBulk { b, act, k, m, r, n } => {
                let k = key(*k);
                let pick_it = |x: &K| {
                    let mut h = std::hash::DefaultHasher::new();
                    std::hash::Hash::hash(&format!("{x:?}"), &mut h);
                    (std::hash::Hasher::finish(&h) % u64::from(*m)) as u8 == *r
                };
                match act {
                    0 => {
                        let x = pick(&mut sets, *b).split_off(&k);
                        let y = pick(&mut smodels, *b).split_off(&k);
                        *pick(&mut sets, !*b) = x;
                        *pick(&mut smodels, !*b) = y;
                    }
                    1 => {
                        let (x, y) = if *b { (&mut sets.1, &mut sets.0) } else { (&mut sets.0, &mut sets.1) };
                        x.append(y);
                        let (x, y) = if *b { (&mut smodels.1, &mut smodels.0) } else { (&mut smodels.0, &mut smodels.1) };
                        x.append(y);
                    }
                    2 => {
                        pick(&mut sets, *b).retain(|x| !pick_it(x));
                        pick(&mut smodels, *b).retain(|x| !pick_it(x));
                    }
                    3 => {
                        for j in 0..*n {
                            let (x, y) = if *m % 2 == 0 {
                                (pick(&mut sets, *b).pop_first(), pick(&mut smodels, *b).pop_first())
                            } else {
                                (pick(&mut sets, *b).pop_last(), pick(&mut smodels, *b).pop_last())
                            };
                            ensure(eqd(&x, &y), "umodel__mismatch__set_pop", || at(format!("pop {j}: {x:?} vs {y:?}")))?;
                            if y.is_none() {
                                break;
                            }
                        }
                        let (x, y) = (pick(&mut sets, *b).first().cloned(), pick(&mut smodels, *b).first().cloned());
                        ensure(eqd(&x, &y), "umodel__mismatch__set_first_last", || at(format!("first {x:?} vs {y:?}")))?;
                        let (x, y) = (pick(&mut sets, *b).last().cloned(), pick(&mut smodels, *b).last().cloned());
                        ensure(eqd(&x, &y), "umodel__mismatch__set_first_last", || at(format!("last {x:?} vs {y:?}")))?;
                    }
                    4 => {
                        let x: Vec<K> = pick(&mut sets, *b).extract_if((Bound::Included(k.clone()), Bound::Unbounded), |x| pick_it(x)).take(*n as usize).collect();
                        let y: Vec<K> = pick(&mut smodels, *b).extract_if((Bound::Included(k), Bound::Unbounded), |x| pick_it(x)).take(*n as usize).collect();
                        ensure(eqd(&x, &y), "umodel__mismatch__set_extract_if", || at(format!("extracted {} vs {}", x.len(), y.len())))?;
                    }
                    5 => {
                        let set = std::mem::replace(pick(&mut sets, *b), new_set());
                        let model = std::mem::take(pick(&mut smodels, *b));
                        let walk: Vec<bool> = (0..*n % 64).map(|j| (u64::from(j) ^ u64::from(*r)) % 3 == 0).collect();
                        let want: Vec<K> = model.into_iter().collect();
                        if *m % 2 == 0 {
                            walk_check(set.into_iter(), want, &walk, true).map_err(|e| wf("set_into_iter", e))?;
                        } else {
                            // dropped part way
                            let mut it = set.into_iter();
                            for j in 0..walk.len().min(want.len()) {
                                let x = it.next();
                                ensure(eqd(&x.as_ref(), &Some(&want[j])), "umodel__mismatch__set_into_iter", || at(format!("item {j}: {x:?}")))?;
                            }
                        }
                    }
                    6 => {
                        let copy = pick(&mut sets, *b).clone();
                        if let Some(why) = same_set(&copy, pick(&mut smodels, *b)) {
                            return Err(fail("umodel__mismatch__set_clone", at(why)));
                        }
                        invariants_set(&copy, &at("the set's clone".into()))?;
                        *pick(&mut sets, *b) = copy;
                    }
                    _ => {
                        pick(&mut sets, *b).clear();
                        pick(&mut smodels, *b).clear();
                    }
                }
            }
            Op::Extremes { b, lo, hi, j } => {
                let j = usize::from(*j);
                let (lo, hi) = bounds::<K>(*lo, *hi, mode);
                let r = (lo.clone(), hi.clone());
                let (map, model) = (pick(&mut maps, *b), pick(&mut models, *b));
                let kv = |x: Option<(&K, &V)>| x.map(|(k, v)| (k.clone(), v.n()));
                let kw = |x: Option<(&K, &u64)>| x.map(|(k, v)| (k.clone(), *v));
                let checks: Vec<(&str, String, String)> = vec![
                    ("iter.last", format!("{:?}", kv(map.iter().last())), format!("{:?}", kw(model.iter().last()))),
                    ("iter.nth", format!("{:?}", kv(map.iter().nth(j))), format!("{:?}", kw(model.iter().nth(j)))),
                    ("iter.nth_back", format!("{:?}", kv(map.iter().nth_back(j))), format!("{:?}", kw(model.iter().nth_back(j)))),
                    ("iter.count", format!("{}", map.iter().count()), format!("{}", model.iter().count())),
                    ("keys.last/min/max", format!("{:?}", (map.keys().last(), map.keys().min(), map.keys().max())), format!("{:?}", (model.keys().last(), model.keys().min(), model.keys().max()))),
                    ("keys.nth", format!("{:?}", map.keys().nth(j)), format!("{:?}", model.keys().nth(j))),
                    ("values.last/nth", format!("{:?}", (map.values().last().map(|v| v.n()), map.values().nth(j).map(|v| v.n()))), format!("{:?}", (model.values().last(), model.values().nth(j)))),
                    ("range.last", format!("{:?}", kv(map.range(r.clone()).last())), format!("{:?}", kw(model.range(r.clone()).last()))),
                    ("range.min/max", format!("{:?}", (kv(map.range(r.clone()).min()), kv(map.range(r.clone()).max()))), format!("{:?}", (kw(model.range(r.clone()).min()), kw(model.range(r.clone()).max())))),
                    ("range.nth", format!("{:?}", (kv(map.range(r.clone()).nth(j)), kv(map.range(r.clone()).nth_back(j)))), format!("{:?}", (kw(model.range(r.clone()).nth(j)), kw(model.range(r.clone()).nth_back(j))))),
                    ("range.count", format!("{}", map.range(r.clone()).count()), format!("{}", model.range(r.clone()).count())),
                    ("iter.clone", {
                        let mut it = map.range(r.clone());
                        let _ = it.nth(j % 5);
                        let c = it.clone();
                        format!("{:?} {:?}", it.map(|(k, v)| (k.clone(), v.n())).collect::<Vec<_>>(), c.rev().map(|(k, v)| (k.clone(), v.n())).collect::<Vec<_>>())
                    }, {
                        let mut it = model.range(r.clone());
                        let _ = it.nth(j % 5);
                        let c = it.clone();
                        format!("{:?} {:?}", it.map(|(k, v)| (k.clone(), *v)).collect::<Vec<_>>(), c.rev().map(|(k, v)| (k.clone(), *v)).collect::<Vec<_>>())
                    }),
                ];
                for (what, x, y) in checks {
                    ensure(eqd(&x, &y), "umodel__mismatch__iter_methods", || at(format!("{what}: {x} vs {y}")))?;
                }
                let mx: Vec<(String, String)> = {
                    let (x, y) = (map.iter_mut().last().map(|(k, v)| (k.clone(), v.n())), model.iter_mut().last().map(|(k, v)| (k.clone(), *v)));
                    let (x2, y2) = (map.values_mut().nth(j).map(|v| v.n()), model.values_mut().nth(j).map(|v| *v));
                    let (x3, y3) = (map.range_mut(r.clone()).nth_back(j).map(|(k, v)| (k.clone(), v.n())), model.range_mut(r.clone()).nth_back(j).map(|(k, v)| (k.clone(), *v)));
                    let (x4, y4) = (map.iter_mut().count(), model.iter_mut().count());
                    vec![(format!("{x:?} {x2:?} {x3:?} {x4}"), format!("{y:?} {y2:?} {y3:?} {y4}"))]
                };
                for (x, y) in mx {
                    ensure(eqd(&x, &y), "umodel__mismatch__iter_mut_methods", || at(format!("last/nth/nth_back/count: {x} vs {y}")))?;
                }
                let (set, sm) = (pick(&mut sets, *b), pick(&mut smodels, *b));
                let x = format!("{:?}", (set.iter().last(), set.iter().min(), set.iter().max(), set.iter().nth(j), set.iter().nth_back(j), set.iter().count()));
                let y = format!("{:?}", (sm.iter().last(), sm.iter().min(), sm.iter().max(), sm.iter().nth(j), sm.iter().nth_back(j), sm.iter().count()));
                ensure(eqd(&x, &y), "umodel__mismatch__set_iter_methods", || at(format!("iter last/min/max/nth/nth_back/count: {x} vs {y}")))?;
                let x = format!("{:?}", (set.range(r.clone()).last(), set.range(r.clone()).min(), set.range(r.clone()).max(), set.range(r.clone()).nth(j), set.range(r.clone()).count()));
                let y = format!("{:?}", (sm.range(r.clone()).last(), sm.range(r.clone()).min(), sm.range(r.clone()).max(), sm.range(r.clone()).nth(j), sm.range(r.clone()).count()));
                ensure(eqd(&x, &y), "umodel__mismatch__set_iter_methods", || at(format!("range last/min/max/nth/count: {x} vs {y}")))?;
                let (x0, x1, m0, m1) = if *b { (&sets.1, &sets.0, &smodels.1, &smodels.0) } else { (&sets.0, &sets.1, &smodels.0, &smodels.1) };
                let x = format!(
                    "{:?}",
                    (
                        (x0.difference(x1).min(), x0.difference(x1).count(), x0.difference(x1).last(), x0.difference(x1).nth(j)),
                        (x0.intersection(x1).min(), x0.intersection(x1).count(), x0.intersection(x1).last(), x0.intersection(x1).max()),
                        (x0.union(x1).min(), x0.union(x1).count(), x0.union(x1).last(), x0.union(x1).max()),
                        (x0.symmetric_difference(x1).min(), x0.symmetric_difference(x1).count(), x0.symmetric_difference(x1).last()),
                    )
                );
                let y = format!(
                    "{:?}",
                    (
                        (m0.difference(m1).min(), m0.difference(m1).count(), m0.difference(m1).last(), m0.difference(m1).nth(j)),
                        (m0.intersection(m1).min(), m0.intersection(m1).count(), m0.intersection(m1).last(), m0.intersection(m1).max()),
                        (m0.union(m1).min(), m0.union(m1).count(), m0.union(m1).last(), m0.union(m1).max()),
                        (m0.symmetric_difference(m1).min(), m0.symmetric_difference(m1).count(), m0.symmetric_difference(m1).last()),
                    )
                );
                ensure(eqd(&x, &y), "umodel__mismatch__set_algebra_methods", || at(format!("min/count/last/..: {x} vs {y}")))?;
                let (mut dx, mut dy) = (x0.difference(x1), m0.difference(m1));
                let (_, _) = (dx.nth(j % 4), dy.nth(j % 4));
                let (cx, cy) = (dx.clone(), dy.clone());
                let x = format!("{:?} {:?}", dx.collect::<Vec<_>>(), cx.collect::<Vec<_>>());
                let y = format!("{:?} {:?}", dy.collect::<Vec<_>>(), cy.collect::<Vec<_>>());
                ensure(eqd(&x, &y), "umodel__mismatch__set_algebra_methods", || at(format!("difference cloned part way: {x} vs {y}")))?;
                let (mut ix, mut iy) = (x0.intersection(x1), m0.intersection(m1));
                let (_, _) = (ix.nth(j % 4), iy.nth(j % 4));
                let (cx, cy) = (ix.clone(), iy.clone());
                let x = format!("{:?} {:?}", ix.collect::<Vec<_>>(), cx.collect::<Vec<_>>());
                let y = format!("{:?} {:?}", iy.collect::<Vec<_>>(), cy.collect::<Vec<_>>());
                ensure(eqd(&x, &y), "umodel__mismatch__set_algebra_methods", || at(format!("intersection cloned part way: {x} vs {y}")))?;
            }
            Op::SMany { b, ks, insert } => {
                let (set, model) = (pick(&mut sets, *b), pick(&mut smodels, *b));
                let ks = ks.map(&key);
                if *insert {
                    let x = set.insert_many(ks.clone());
                    let y: [bool; 4] = std::array::from_fn(|j| model.insert(ks[j].clone()));
                    ensure(eqd(&x, &y), "umodel__mismatch__set_insert_many", || at(format!("{x:?} vs {y:?}")))?;
                } else {
                    let x = set.contains_many([&ks[0], &ks[1], &ks[2], &ks[3]]);
                    let y: [bool; 4] = std::array::from_fn(|j| model.contains(&ks[j]));
                    ensure(eqd(&x, &y), "umodel__mismatch__set_contains_many", || at(format!("{x:?} vs {y:?}")))?;
                }
            }
        }
        // After every step: the whole of each, and the values alive.
        for (side, (map, model)) in [(&maps.0, &models.0), (&maps.1, &models.1)].into_iter().enumerate() {
            if let Some(why) = same_map(map, model) {
                return Err(fail(&format!("umodel__mismatch__{}", area_of(op)), at(format!("map {side} after it: {why}"))));
            }
            invariants_map(map, &at(format!("map {side} after it")))?;
        }
        for (side, (set, model)) in [(&sets.0, &smodels.0), (&sets.1, &smodels.1)].into_iter().enumerate() {
            if let Some(why) = same_set(set, model) {
                return Err(fail(&format!("umodel__mismatch__{}", area_of(op)), at(format!("set {side} after it: {why}"))));
            }
            invariants_set(set, &at(format!("set {side} after it")))?;
        }
        let want = (models.0.len() + models.1.len()) as i64;
        ensure(live() == want, &format!("umodel__leak__{}", area_of(op)), || at(format!("{} values alive, {want} in the maps", live())))?;
    }
    classify(algebra_hits, "set_algebra_nonempty");
    classify(bomb_fired, "clone_panicked");
    classify(entries, "entry_api");
    classify(models.0.len() + models.1.len() >= 1000, "maps_1k");
    Ok(())
}

/// The signature's area for an op (no values in it).
fn area_of(op: &Op) -> &'static str {
    match op {
        Op::Insert { .. } => "insert",
        Op::Remove { .. } => "remove",
        Op::GetMut { .. } => "get_mut",
        Op::Entry { .. } => "entry",
        Op::End { .. } => "first_last_entry",
        Op::Walk { kind: 0..=3, .. } => "walk",
        Op::Walk { .. } => "walk_mut",
        Op::Consume { .. } => "into_iter",
        Op::Clone { .. } => "clone",
        Op::SplitOff { .. } => "split_off",
        Op::Append { .. } => "append",
        Op::Retain { .. } => "retain",
        Op::ExtractIf { .. } => "extract_if",
        Op::Pop { .. } => "pop",
        Op::Clear { .. } => "clear",
        Op::Extend { .. } => "extend",
        Op::Compare { .. } => "compare",
        Op::Many { .. } => "many",
        Op::Swap => "swap",
        Op::SOne { .. } => "set_one",
        Op::SFromMap { .. } => "set_extend",
        Op::SAlgebra { .. } => "set_algebra",
        Op::SRel => "set_relation",
        Op::SWalk { .. } => "set_walk",
        Op::SBulk { act: 0, .. } => "set_split_off",
        Op::SBulk { act: 1, .. } => "set_append",
        Op::SBulk { act: 2 | 4, .. } => "set_retain",
        Op::SBulk { act: 5, .. } => "set_into_iter",
        Op::SBulk { .. } => "set_bulk",
        Op::SMany { .. } => "set_many",
        Op::Extremes { .. } => "iter_methods",
    }
}

/// One of the entry API's paths; both sides give what they saw, printed.
fn entry_op<K: UKey, V: UVal, S: InlineKeyStrategy<K>>(map: &mut Map<K, V, S>, model: &mut BTreeMap<K, u64>, k: K, act: u8, v: u64) -> (String, String) {
    use std::collections::btree_map::Entry as E;
    match act % 9 {
        0 => {
            let x = map.entry(k.clone()).or_insert(V::new(v)).n();
            let y = *model.entry(k).or_insert(v);
            (format!("{x}"), format!("{y}"))
        }
        1 => {
            let x = map.entry(k.clone()).or_insert_with(|| V::new(v)).n();
            let y = *model.entry(k).or_insert_with(|| v);
            (format!("{x}"), format!("{y}"))
        }
        2 => {
            let x = map.entry(k.clone()).or_insert_with_key(|key| V::new(v ^ (format!("{key:?}").len() as u64))).n();
            let y = *model.entry(k).or_insert_with_key(|key| v ^ (format!("{key:?}").len() as u64));
            (format!("{x}"), format!("{y}"))
        }
        3 => {
            let x = map.entry(k.clone()).and_modify(|e| e.bump(v)).or_default().n();
            let y = *model.entry(k).and_modify(|e| *e = e.wrapping_add(v)).or_default();
            (format!("{x}"), format!("{y}"))
        }
        4 => {
            let x = match map.entry(k.clone()) {
                Entry::Occupied(o) => {
                    let (k, val) = o.remove_entry();
                    format!("removed {k:?} {}", val.n())
                }
                Entry::Vacant(e) => {
                    let key = e.key().clone();
                    format!("inserted {key:?} {}", e.insert(V::new(v)).n())
                }
            };
            let y = match model.entry(k) {
                E::Occupied(o) => {
                    let (k, val) = o.remove_entry();
                    format!("removed {k:?} {val}")
                }
                E::Vacant(e) => {
                    let key = e.key().clone();
                    format!("inserted {key:?} {}", e.insert(v))
                }
            };
            (x, y)
        }
        5 => {
            let x = match map.entry(k.clone()) {
                Entry::Occupied(mut o) => {
                    let old = o.insert(V::new(v)).n();
                    format!("{:?} was {old} now {}", o.key(), o.get().n())
                }
                Entry::Vacant(e) => format!("vacant {:?}", e.into_key()),
            };
            let y = match model.entry(k) {
                E::Occupied(mut o) => {
                    let old = o.insert(v);
                    format!("{:?} was {old} now {}", o.key(), o.get())
                }
                E::Vacant(e) => format!("vacant {:?}", e.into_key()),
            };
            (x, y)
        }
        6 => {
            let x = match map.entry(k.clone()).insert_entry(V::new(v)) {
                o if v % 2 == 0 => format!("removed {}", o.remove().n()),
                o => format!("{:?} {}", o.key(), o.get().n()),
            };
            let y = match model.entry(k).insert_entry(v) {
                o if v % 2 == 0 => format!("removed {}", o.remove()),
                o => format!("{:?} {}", o.key(), o.get()),
            };
            (x, y)
        }
        7 => {
            let x = match map.entry(k.clone()) {
                Entry::Occupied(mut o) => {
                    o.get_mut().bump(v);
                    let r = o.into_mut();
                    r.bump(1);
                    format!("occupied {}", r.n())
                }
                Entry::Vacant(e) => {
                    let o = e.insert_entry(V::new(v));
                    format!("vacant then {:?} {}", o.key(), o.get().n())
                }
            };
            let y = match model.entry(k) {
                E::Occupied(mut o) => {
                    *o.get_mut() = o.get().wrapping_add(v);
                    let r = o.into_mut();
                    *r = r.wrapping_add(1);
                    format!("occupied {r}")
                }
                E::Vacant(e) => {
                    let o = e.insert_entry(v);
                    format!("vacant then {:?} {}", o.key(), o.get())
                }
            };
            (x, y)
        }
        _ => {
            let x = format!("{:?}", map.entry(k.clone()).key());
            let y = format!("{:?}", model.entry(k).key());
            (x, y)
        }
    }
}

fn end_op<K: UKey, V: UVal, S: InlineKeyStrategy<K>>(map: &mut Map<K, V, S>, model: &mut BTreeMap<K, u64>, last: bool, act: u8, v: u64) -> (String, String) {
    let x = match if last { map.last_entry() } else { map.first_entry() } {
        None => "none".to_string(),
        Some(mut o) => match act % 4 {
            0 => format!("removed {}", o.remove().n()),
            1 => {
                let (k, val) = o.remove_entry();
                format!("removed {k:?} {}", val.n())
            }
            2 => {
                let old = o.insert(V::new(v)).n();
                format!("{:?} was {old}", o.key())
            }
            _ => {
                o.get_mut().bump(v);
                format!("{:?} now {}", o.key(), o.get().n())
            }
        },
    };
    let y = match if last { model.last_entry() } else { model.first_entry() } {
        None => "none".to_string(),
        Some(mut o) => match act % 4 {
            0 => format!("removed {}", o.remove()),
            1 => {
                let (k, val) = o.remove_entry();
                format!("removed {k:?} {val}")
            }
            2 => {
                let old = o.insert(v);
                format!("{:?} was {old}", o.key())
            }
            _ => {
                *o.get_mut() = o.get().wrapping_add(v);
                format!("{:?} now {}", o.key(), o.get())
            }
        },
    };
    (x, y)
}

// ---------- the same ops on bbtree::BBTreeMap, one thread ----------

type SMap<K, V, S> = bbtree::BBTreeMap<K, V, S, Pages>;

#[allow(unused_variables)]
fn invariants_sync<K: UKey, V: UVal, S: InlineKeyStrategy<K>>(map: &mut SMap<K, V, S>, len: usize, at: &str) -> Result<(), Fail> {
    #[cfg(fuzzing)]
    {
        let r = catch_unwind(AssertUnwindSafe(|| map.check_invariants(len)));
        if let Err(p) = r {
            let msg = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()));
            return Err(fail("umodel__invariant__syncmap", format!("{at}: {msg:?}")));
        }
    }
    Ok(())
}

fn scan_all<K: UKey, V: UVal, S: InlineKeyStrategy<K>>(map: &SMap<K, V, S>) -> Vec<(K, u64)> {
    let mut out = Vec::new();
    map.scan::<K, ()>(.., |k, v| {
        out.push((k.clone(), v.n()));
        std::ops::ControlFlow::Continue(())
    });
    out
}

/// The map ops of a case on the latched map, used by one thread: `read`,
/// `update`, `get_mut`, `update_or_insert`, `get_or_insert`, `remove_if`,
/// `scan`/`scan_rev` with early stops, `range_cloned` walks, `iter_mut`,
/// `into_iter` dropped part way, `clear`, `extend` and the batch calls.
/// Set ops and the ops the latched map lacks (split_off, append, retain,
/// extract_if, clone) are skipped.
/// The std traits the iterators and collections implement on their own
/// instead of taking the default: `last`/`min`/`max` of every owning and
/// mutable walk (overridden to step from the back), `ExactSizeIterator::len`,
/// `partial_cmp`, `is_empty`, and `Debug` (of the collections, and of a
/// consuming walk part way, which shows what is left), each against std's.
/// The variant comes from `pick`; maps past 3000 entries skip the clones.
fn glue<K: UKey, V: UVal, S: InlineKeyStrategy<K> + Default>(
    maps: &mut (Map<K, V, S>, Map<K, V, S>),
    models: &(BTreeMap<K, u64>, BTreeMap<K, u64>),
    sets: &(Set<K, S>, Set<K, S>),
    smodels: &(BTreeSet<K>, BTreeSet<K>),
    pick: u64,
) -> Result<(), (&'static str, String)> {
    let chk = |ok: bool, area: &'static str, msg: &dyn Fn() -> String| if ok { Ok(()) } else { Err((area, msg())) };
    let x = (maps.0.partial_cmp(&maps.1), sets.0.partial_cmp(&sets.1), sets.0.is_empty(), sets.1.is_empty());
    let y = (models.0.partial_cmp(&models.1), smodels.0.partial_cmp(&smodels.1), smodels.0.is_empty(), smodels.1.is_empty());
    chk(x == y, "partial_cmp", &|| format!("{x:?} vs {y:?}"))?;
    let (map, model) = if pick & 1 == 0 { (&mut maps.0, &models.0) } else { (&mut maps.1, &models.1) };
    let (set, smodel) = if pick & 1 == 0 { (&sets.0, &smodels.0) } else { (&sets.1, &smodels.1) };
    if model.len() <= 300 && smodel.len() <= 300 {
        let (a, b) = (format!("{map:?}"), format!("{model:?}"));
        chk(a == b, "debug", &|| format!("map {a} vs {b}"))?;
        let (a, b) = (format!("{set:?}"), format!("{smodel:?}"));
        chk(a == b, "debug", &|| format!("set {a} vs {b}"))?;
    }
    let kv = |(k, v): (&K, &u64)| (k.clone(), *v);
    let want_first = model.iter().next().map(kv);
    let want_last = model.iter().next_back().map(kv);
    // iter_mut / values_mut: no value changes, so the map stays as it was.
    let x = (map.iter_mut().len(), map.iter_mut().min().map(|(k, v)| (k.clone(), v.n())), map.iter_mut().max().map(|(k, v)| (k.clone(), v.n())));
    let y = (model.len(), want_first.clone(), want_last.clone());
    chk(eqd(&x, &y), "iter_mut_glue", &|| format!("len/min/max {x:?} vs {y:?}"))?;
    let x = (map.iter_mut().last().map(|(k, v)| (k.clone(), v.n())), map.values_mut().last().map(|v| v.n()), map.values_mut().len());
    let y = (want_last.clone(), want_last.as_ref().map(|p| p.1), model.len());
    chk(eqd(&x, &y), "iter_mut_glue", &|| format!("last/values_mut last/len {x:?} vs {y:?}"))?;
    if model.len() > 3000 {
        return Ok(());
    }
    let skip = ((pick >> 1) % 5) as usize;
    let want: Vec<(K, u64)> = model.iter().skip(skip).map(kv).collect();
    match (pick >> 4) % 6 {
        0 => {
            let mut it = map.clone().into_iter();
            for _ in 0..skip {
                it.next();
            }
            let a = format!("{:?}", it.map(|(k, v)| (k, v.n())).collect::<Vec<_>>());
            let mut sit = model.clone().into_iter();
            for _ in 0..skip {
                sit.next();
            }
            let b = format!("{:?}", sit.collect::<Vec<_>>());
            chk(a == b, "into_iter_glue", &|| format!("rest {a} vs {b}"))?;
            let mut it = map.clone().into_iter();
            let mut sit = model.clone().into_iter();
            for _ in 0..skip {
                it.next();
                sit.next();
            }
            let (a, b) = (format!("{it:?}"), format!("{sit:?}"));
            chk(a == b, "debug", &|| format!("into_iter part way {a} vs {b}"))?;
        }
        1 => {
            let mut it = map.clone().into_iter();
            for _ in 0..skip {
                it.next();
            }
            let x = it.last().map(|(k, v)| (k, v.n()));
            chk(eqd(&x, &want.last().cloned()), "into_iter_glue", &|| format!("last {x:?} vs {:?}", want.last()))?;
        }
        2 => {
            let mut it = map.clone().into_keys();
            for _ in 0..skip {
                it.next();
            }
            let x = (it.size_hint().0, it.last());
            let y = (want.len(), want.last().map(|p| p.0.clone()));
            chk(eqd(&x, &y), "into_iter_glue", &|| format!("into_keys len/last {x:?} vs {y:?}"))?;
        }
        3 => {
            let mut it = map.clone().into_keys();
            for _ in 0..skip {
                it.next();
            }
            let x = it.min();
            let y = want.first().map(|p| p.0.clone());
            chk(eqd(&x, &y), "into_iter_glue", &|| format!("into_keys min {x:?} vs {y:?}"))?;
            let mut it = map.clone().into_keys();
            for _ in 0..skip {
                it.next();
            }
            let x = it.max();
            let y = want.last().map(|p| p.0.clone());
            chk(eqd(&x, &y), "into_iter_glue", &|| format!("into_keys max {x:?} vs {y:?}"))?;
        }
        4 => {
            let mut it = map.clone().into_values();
            for _ in 0..skip {
                it.next();
            }
            let x = it.last().map(|v| v.n());
            let y = want.last().map(|p| p.1);
            chk(x == y, "into_iter_glue", &|| format!("into_values last {x:?} vs {y:?}"))?;
        }
        _ => {
            let mut it = map.clone().into_keys();
            let mut sit = model.clone().into_keys();
            for _ in 0..skip {
                it.next();
                sit.next();
            }
            let (a, b) = (format!("{it:?}"), format!("{sit:?}"));
            chk(a == b, "debug", &|| format!("into_keys part way {a} vs {b}"))?;
            let mut it = map.clone().into_values();
            let mut sit = model.clone().into_values();
            for _ in 0..skip {
                it.next();
                sit.next();
            }
            let a = format!("{:?}", it.map(|v| v.n()).collect::<Vec<_>>());
            let b = format!("{:?}", sit.collect::<Vec<_>>());
            chk(a == b, "into_iter_glue", &|| format!("into_values rest {a} vs {b}"))?;
        }
    }
    Ok(())
}

/// The same model under the typemx family: other key and value types, so
/// failures there get signatures of their own.
pub fn tm<K: UKey, V: UVal, S: InlineKeyStrategy<K> + Default>(case: &Case) -> Result<(), Fail> {
    run::<K, V, S>(case).map_err(retag)
}
pub fn tm_sync<K: UKey + Send + Sync, V: UVal + Send + Sync, S: InlineKeyStrategy<K> + Default>(case: &Case) -> Result<(), Fail> {
    run_sync::<K, V, S>(case).map_err(retag)
}
fn retag(mut f: Fail) -> Fail {
    f.sig = f.sig.replacen("umodel__", "typemx__", 1);
    f
}

pub fn run_sync<K: UKey + Send + Sync, V: UVal, S: InlineKeyStrategy<K> + Default>(case: &Case) -> Result<(), Fail> {
    LIVE.with(|c| c.set(0));
    PAGES.with(|c| c.set(0));
    CLONE_BOMB.with(|c| c.set(0));
    TAG.with(|t| t.set(0));
    TAGGED.with(|t| t.set(K::TAGGED));
    run_sync_inner::<K, V, S>(case)?;
    ensure(live() == 0, "umodel__leak__sync_values", || format!("{} values alive after the maps dropped", live()))?;
    ensure(pages() == 0, "umodel__leak__sync_pages", || format!("{} pages out after the maps dropped", pages()))
}

fn run_sync_inner<K: UKey + Send + Sync, V: UVal, S: InlineKeyStrategy<K> + Default>(case: &Case) -> Result<(), Fail> {
    use std::ops::ControlFlow::{Break, Continue};
    let mode = case.mode;
    let key = |r: u128| K::of(r, mode);
    let new = || SMap::<K, V, S>::new_in(Pages);
    let mut maps = (new(), new());
    let mut models: (BTreeMap<K, u64>, BTreeMap<K, u64>) = (BTreeMap::new(), BTreeMap::new());
    for (side, seed) in [(false, &case.a), (true, &case.b)] {
        for raw in seed.raw_keys() {
            let (k, v) = (key(raw), seed.value_of(raw));
            let _ = pick(&mut maps, side).insert(k.clone(), V::new(v));
            pick(&mut models, side).insert(k, v);
        }
    }
    let refused = |what: &str| fail("umodel__mismatch__sync_refused", format!("{what}: a page refused with an allocator that never refuses"));
    for (i, op) in case.ops.iter().enumerate() {
        TAG.with(|t| t.set(i as u32 + 1));
        let at = |why: String| format!("op {i} {op:?}: {why}");
        let wf = |area: &str, (kind, msg): (&str, String)| fail(&format!("umodel__{kind}__{area}"), at(msg));
        match op {
            Op::Insert { b, k, v } => {
                let x = pick(&mut maps, *b).insert(key(*k), V::new(*v)).map_err(|_| refused("insert"))?.map(|v| v.n());
                let y = pick(&mut models, *b).insert(key(*k), *v);
                ensure(eqd(&x, &y), "umodel__mismatch__sync_insert", || at(format!("{x:?} vs {y:?}")))?;
            }
            Op::Remove { b, k, entry } => {
                let k = key(*k);
                let (map, model) = (pick(&mut maps, *b), pick(&mut models, *b));
                if *entry {
                    let x = map.remove_if(&k, |_, v| v.n() % 3 != 0).map(|(k, v)| (k, v.n()));
                    let y = if model.get(&k).is_some_and(|v| v % 3 != 0) { model.remove_entry(&k) } else { None };
                    ensure(eqd(&x, &y), "umodel__mismatch__sync_remove_if", || at(format!("{x:?} vs {y:?}")))?;
                } else {
                    let (x, y) = (map.remove(&k).map(|v| v.n()), model.remove(&k));
                    ensure(eqd(&x, &y), "umodel__mismatch__sync_remove", || at(format!("{x:?} vs {y:?}")))?;
                }
            }
            Op::GetMut { b, k, d } => {
                let k = key(*k);
                let (map, model) = (pick(&mut maps, *b), pick(&mut models, *b));
                let x = if d % 2 == 0 {
                    map.get_mut(&k).map(|v| {
                        v.bump(*d);
                        v.n()
                    })
                } else {
                    map.update(&k, |_, v| {
                        v.bump(*d);
                        v.n()
                    })
                };
                let y = model.get_mut(&k).map(|v| {
                    *v = v.wrapping_add(*d);
                    *v
                });
                ensure(eqd(&x, &y), "umodel__mismatch__sync_update", || at(format!("{x:?} vs {y:?}")))?;
            }
            Op::Entry { b, k, act, v } => {
                let k = key(*k);
                let (map, model) = (pick(&mut maps, *b), pick(&mut models, *b));
                let (x, y) = match act % 4 {
                    0 => {
                        let x = map
                            .update_or_insert(k.clone(), V::new(*v), |_, s, new| {
                                s.bump(new.n());
                                s.n()
                            })
                            .map_err(|_| refused("update_or_insert"))?;
                        let y = match model.get_mut(&k) {
                            Some(s) => {
                                *s = s.wrapping_add(*v);
                                Some(*s)
                            }
                            None => {
                                model.insert(k, *v);
                                None
                            }
                        };
                        (x, y)
                    }
                    1 => {
                        let x = map.get_or_insert(k.clone(), V::new(*v), |_, s| s.n()).map_err(|_| refused("get_or_insert"))?;
                        (Some(x), Some(*model.entry(k).or_insert(*v)))
                    }
                    2 => (map.read(&k, |_, s| s.n()), model.get(&k).copied()),
                    _ => (map.get_cloned(&k).map(|s| s.n()).filter(|_| map.contains_key(&k)), model.get(&k).copied()),
                };
                ensure(eqd(&x, &y), "umodel__mismatch__sync_entry", || at(format!("{x:?} vs {y:?}")))?;
            }
            Op::End { b, last, act, .. } => {
                let (map, model) = (pick(&mut maps, *b), pick(&mut models, *b));
                let x = if *last { map.scan_rev::<K, _>(.., |k, v| Break((k.clone(), v.n()))) } else { map.scan::<K, _>(.., |k, v| Break((k.clone(), v.n()))) };
                let y = if *last { model.last_key_value() } else { model.first_key_value() }.map(|(k, v)| (k.clone(), *v));
                ensure(eqd(&x, &y), "umodel__mismatch__sync_scan_end", || at(format!("{x:?} vs {y:?}")))?;
                if let (Some((k, _)), true) = (y, act % 2 == 0) {
                    map.remove(&k);
                    model.remove(&k);
                }
            }
            Op::Walk { b, kind, lo, hi, walk, d } => {
                let (map, model) = (pick(&mut maps, *b), pick(&mut models, *b));
                let (lo, hi) = bounds::<K>(*lo, *hi, mode);
                match kind {
                    0..=2 => {
                        // scans, stopping after walk.len() entries when walk[0]
                        let stop = if walk.first() == Some(&true) { walk.len() } else { usize::MAX };
                        let rev = *kind == 1;
                        let mut got = Vec::new();
                        let f = |k: &K, v: &V| {
                            if got.len() >= stop {
                                return Break(got.len());
                            }
                            got.push((k.clone(), v.n()));
                            Continue(())
                        };
                        let broke = if rev { map.scan_rev((lo.clone(), hi.clone()), f) } else { map.scan((lo.clone(), hi.clone()), f) };
                        let all: Vec<(K, u64)> = model.range((lo, hi)).map(|(k, v)| (k.clone(), *v)).collect();
                        let mut want: Vec<_> = if rev { all.iter().rev().cloned().collect() } else { all.clone() };
                        let want_break = (want.len() > stop).then_some(stop);
                        want.truncate(stop);
                        ensure(got == want && broke == want_break, if rev { "umodel__mismatch__sync_scan_rev" } else { "umodel__mismatch__sync_scan" }, || {
                            at(format!("{} entries (break {broke:?}), want {} (break {want_break:?})", got.len(), want.len()))
                        })?;
                    }
                    3 => walk_check(
                        map.range_cloned((lo.clone(), hi.clone())).map(|(k, v)| (k, v.n())),
                        model.range((lo, hi)).map(|(k, v)| (k.clone(), *v)).collect(),
                        walk,
                        false,
                    )
                    .map_err(|e| wf("sync_range_cloned", e))?,
                    _ => {
                        let want = model.iter_mut().map(|(k, v)| (k.clone(), std::mem::replace(v, v.wrapping_add(*d)))).collect();
                        let ours = map.iter_mut().map(|(k, v)| {
                            let old = v.n();
                            v.bump(*d);
                            (k.clone(), old)
                        });
                        walk_check(ours, want, walk, false).map_err(|e| wf("sync_iter_mut", e))?;
                    }
                }
            }
            Op::Consume { b, walk, drain, .. } => {
                let map = std::mem::replace(pick(&mut maps, *b), new());
                let model = std::mem::take(pick(&mut models, *b));
                let n = if *drain { usize::MAX } else { walk.len() };
                let want: Vec<(K, u64)> = model.into_iter().collect();
                walk_forward(map.into_iter().map(|(k, v)| (k, v.n())), &want, n)
                    .map_err(|e| wf("sync_into_iter", e))?;
            }
            Op::Pop { b, last, n } => {
                for _ in 0..(*n).min(64) {
                    let (map, model) = (pick(&mut maps, *b), pick(&mut models, *b));
                    let Some((k, _)) = (if *last { model.pop_last() } else { model.pop_first() }) else { break };
                    ensure(map.remove(&k).is_some(), "umodel__mismatch__sync_remove", || at(format!("{k:?} missing")))?;
                }
            }
            Op::Clear { b } => {
                pick(&mut maps, *b).clear();
                pick(&mut models, *b).clear();
            }
            Op::Extend { b, start, stride, len } => {
                let raws: Vec<u128> = (0..u128::from(*len)).map(|j| start.wrapping_add(j * u128::from(*stride))).collect();
                pick(&mut maps, *b).extend(raws.iter().map(|&r| (key(r), V::new(r as u64))));
                pick(&mut models, *b).extend(raws.iter().map(|&r| (key(r), r as u64)));
            }
            Op::Compare { k } => {
                let k = key(*k);
                for (map, model) in [(&maps.0, &models.0), (&maps.1, &models.1)] {
                    let x = (map.is_empty(), map.contains_key(&k), map.read(&k, |key, v| (key.clone(), v.n())));
                    let y = (model.is_empty(), model.contains_key(&k), model.get_key_value(&k).map(|(k, v)| (k.clone(), *v)));
                    ensure(eqd(&x, &y), "umodel__mismatch__sync_read", || at(format!("{x:?} vs {y:?}")))?;
                }
            }
            Op::Many { b, ks, v, insert } => {
                let (map, model) = (pick(&mut maps, *b), pick(&mut models, *b));
                let ks = ks.map(&key);
                if *insert {
                    let entries = std::array::from_fn::<_, 4, _>(|j| (ks[j].clone(), V::new(v.wrapping_add(j as u64))));
                    let x: Vec<Option<u64>> = if v % 2 == 0 {
                        map.insert_many(entries).into_iter().map(|r| r.map(|o| o.map(|v| v.n()))).collect::<Result<_, _>>()
                    } else {
                        map.update_or_insert_many(entries, |_, s, new| {
                            let old = s.n();
                            *s = new;
                            old
                        })
                        .into_iter()
                        .collect::<Result<_, _>>()
                    }
                    .map_err(|_| refused("insert_many"))?;
                    let y: Vec<Option<u64>> = (0..4).map(|j| model.insert(ks[j].clone(), v.wrapping_add(j as u64))).collect();
                    ensure(eqd(&x, &y), "umodel__mismatch__sync_insert_many", || at(format!("{x:?} vs {y:?}")))?;
                } else {
                    let refs = [&ks[0], &ks[1], &ks[2], &ks[3]];
                    let x: Vec<Option<u64>> = if v % 2 == 0 {
                        map.get_many_cloned(refs).iter().map(|o| o.as_ref().map(|v| v.n())).collect()
                    } else {
                        map.read_many(refs, |_, v| v.n()).to_vec()
                    };
                    let y: Vec<Option<u64>> = ks.iter().map(|k| model.get(k).copied()).collect();
                    ensure(eqd(&x, &y), "umodel__mismatch__sync_get_many", || at(format!("{x:?} vs {y:?}")))?;
                }
            }
            Op::Swap => {
                std::mem::swap(&mut maps.0, &mut maps.1);
                std::mem::swap(&mut models.0, &mut models.1);
            }
            _ => continue,
        }
        for side in [false, true] {
            let (map, model) = (pick(&mut maps, side), pick(&mut models, side));
            let ours = scan_all(map);
            if ours.len() != model.len() || !ours.iter().zip(model.iter()).all(|(a, b)| eqd(&(&a.0, &a.1), &b)) {
                let pos = ours.iter().zip(model.iter()).position(|(a, b)| !eqd(&(&a.0, &a.1), &b));
                return Err(fail(&format!("umodel__mismatch__sync_{}", area_of(op)), at(format!("map {side} after it: {} vs {} entries, first difference {pos:?}", ours.len(), model.len()))));
            }
            invariants_sync(map, model.len(), &at(format!("map {side} after it")))?;
        }
        let want = (models.0.len() + models.1.len()) as i64;
        ensure(live() == want, &format!("umodel__leak__sync_{}", area_of(op)), || at(format!("{} values alive, {want} in the maps", live())))?;
    }
    Ok(())
}
