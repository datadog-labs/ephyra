// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! mmapmx: `bbtree-mmap`'s `BBTreeMapFile` and `bbtree::file`'s
//! `FileWriter` / `FileMap` over the key and value types and the input paths
//! the mmap and filemap oracles never take: plain keys (`[u8; N]` of every
//! kind of layout, `i8`, `u16`, `i128`, and the caller's own plain types
//! made disk types by `plain_disk_type!`, under `Auto` and under
//! `WithInlineKey` with their own `InlineKey` impls of width 2, 4 and 8,
//! with and without hash tags and `cmp_from`), values of every plain kind
//! (`f32`/`f64` of any bits, `u128`, zero-sized `[u8; 0]`, odd arrays) and
//! text / bytes; written from std's map, the unsync map, the sync map by its
//! owner (`iter_mut`), owned entries, `&mut` entries and `Cow` bytes; read
//! back by `read`, `mmap` in small and huge pages, `read_range` and
//! `mmap().range()`, answering as std's map does (len, is_empty, iter both
//! ways, get, get_key_value, get_many, ranges walked from both ends).
//!
//! Also the documented refusals: entries that don't ascend strictly make
//! `create` / `write_at` fail with `InvalidInput` "before the file is
//! touched" / "`file` is untouched if they don't ascend", and
//! `FileWriter::from_ascending` fail with `NotAscending` whose `index` is
//! "the index of the first entry whose key is not greater than the key
//! before it". And for plain values, `FileWriter::from(&unsync)`,
//! `from(&mut sync)` ("the file map holds exactly the map's entries") and
//! `from_ascending` write the same bytes, and `FileMap`'s `keys`, `values`,
//! `Debug`, cloned ranges and `rfold` answer as std's.

use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt::Debug;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::ops::Bound;
use std::panic::{AssertUnwindSafe, catch_unwind};

use bbtree::file::{FileMap, FileWriter, PlainBytes, Storable};
use bbtree::{InlineKey, InlineKeyStrategy, InlineWidth, InlineWord, WithInlineKey};
use bbtree_mmap::{BBTreeMapFile, DiskInput, DiskQuery, DiskType, OsPages, plain_disk_type};
use ephyra::{BoxGen, GenExt, classify, gens};
use serde::{Deserialize, Serialize};

use crate::common::{Fail, compare_walk, ensure, fail, range_panics};
use crate::mmap::TempPath;

// ---------------------------------------------------------------- keys

/// Bytes of a plain value, which has no padding (`PlainBytes`).
fn plain_bytes<T: PlainBytes>(t: &T) -> Vec<u8> {
    // SAFETY: PlainBytes: every byte initialized, no padding.
    unsafe { std::slice::from_raw_parts(t as *const T as *const u8, size_of::<T>()) }.to_vec()
}

std::thread_local! {
    /// A broken call premise seen by a user `InlineKey` impl.
    static PREMISE: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

fn premise(what: String) {
    PREMISE.with(|p| {
        let mut p = p.borrow_mut();
        if p.is_none() {
            *p = Some(what);
        }
    });
}

fn take_premise() -> Option<String> {
    PREMISE.with(|p| p.borrow_mut().take())
}

fn fnv(bytes: &[u8]) -> u8 {
    let mut h: u32 = 0x811c_9dc5;
    for &b in bytes {
        h = (h ^ b as u32).wrapping_mul(0x0100_0193);
    }
    (h ^ (h >> 8) ^ (h >> 16) ^ (h >> 24)) as u8
}

/// A plain key of the caller's: `N` bytes, ordered as bytes (its encoding),
/// with its own `InlineKey` impl of the given width and options.
macro_rules! user_key {
    ($name:ident, $n:literal, $width:expr, $tag:literal, $cmp_from:literal, $own_prefix:literal) => {
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
        #[repr(transparent)]
        pub struct $name(pub [u8; $n]);

        // SAFETY: one byte array: every bit pattern, no padding, no pointer.
        unsafe impl PlainBytes for $name {}

        impl InlineKey for $name {
            const WIDTH: InlineWidth = $width;
            const HAS_HASH_TAG: bool = $tag;
            const HAS_CMP_FROM: bool = $cmp_from;

            fn inline_key_at(&self, offset: usize) -> InlineWord {
                self.0.as_slice().inline_key_at(offset)
            }

            fn hash_tag(&self) -> u8 {
                if !$tag {
                    premise(format!("hash_tag called on {} without HAS_HASH_TAG", stringify!($name)));
                }
                fnv(&self.0)
            }

            fn cmp_from(&self, other: &Self, equal_len: usize) -> Ordering {
                if !$cmp_from {
                    premise(format!("cmp_from called on {} without HAS_CMP_FROM", stringify!($name)));
                }
                let e = equal_len.min($n);
                if self.0[..e] != other.0[..e] {
                    premise(format!("cmp_from({:?}, {:?}, {equal_len}): the first equal_len bytes differ", self.0, other.0));
                }
                self.0[e..].cmp(&other.0[e..])
            }

            fn cmp_prefix(&self, prefix: &[u8]) -> Ordering {
                if prefix.len() > 64 {
                    premise(format!("cmp_prefix of {} bytes, past 64", prefix.len()));
                }
                if $own_prefix {
                    let mut padded = self.0.to_vec();
                    padded.resize(prefix.len().max($n), 0);
                    padded[..prefix.len()].cmp(prefix)
                } else {
                    let (words, tail) = prefix.as_chunks::<8>();
                    for (index, word) in words.iter().enumerate() {
                        let window = self.inline_key_at(8 * index);
                        let prefix_word = InlineWord::from_be_bytes(*word);
                        if window != prefix_word {
                            return window.cmp(&prefix_word);
                        }
                    }
                    if tail.is_empty() {
                        return Ordering::Equal;
                    }
                    let dropped_bits = 8 * (8 - tail.len()) as u32;
                    let window = self.inline_key_at(8 * words.len()) >> dropped_bits;
                    window.cmp(&(tail.inline_key_at(0) >> dropped_bits))
                }
            }
        }

        plain_disk_type!($name, WithInlineKey);

        impl MxKey for $name {
            fn from_bytes(b: &[u8]) -> Self {
                let mut k = [0u8; $n];
                k.copy_from_slice(&b[..$n]);
                $name(k)
            }
            fn kbytes(v: &&Self) -> Vec<u8> {
                plain_bytes(*v)
            }
        }
    };
}

user_key!(Uk5W2, 5, InlineWidth::Two, false, false, false);
user_key!(Uk12W2, 12, InlineWidth::Two, true, true, false);
user_key!(Uk24W4, 24, InlineWidth::Four, true, false, true);
user_key!(Uk70W8, 70, InlineWidth::Eight, false, true, false);
user_key!(Uk9W4, 9, InlineWidth::Four, false, true, true);

/// The docs' `UserId`: a u64 under `WithInlineKey`, its inline key the u64's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct UserId(pub u64);
// SAFETY: one u64.
unsafe impl PlainBytes for UserId {}
impl InlineKey for UserId {
    const WIDTH: InlineWidth = u64::WIDTH;
    fn inline_key_at(&self, offset: usize) -> InlineWord {
        self.0.inline_key_at(offset)
    }
}
plain_disk_type!(UserId, WithInlineKey);
impl MxKey for UserId {
    fn from_bytes(b: &[u8]) -> Self {
        UserId(u64::from_be_bytes(b[..8].try_into().unwrap()))
    }
    fn kbytes(v: &&Self) -> Vec<u8> {
        plain_bytes(*v)
    }
}

/// A plain key type: made from a key's bytes (its encoding, big-endian for
/// integers), viewed as itself.
pub trait MxKey: Copy + Ord + Debug + PlainBytes + Send + Sync + 'static + DiskType + DiskQuery<Stored = Self> {
    fn from_bytes(b: &[u8]) -> Self;
    fn kbytes(v: &Self::View<'_>) -> Vec<u8>;
}

impl<const N: usize> MxKey for [u8; N] {
    fn from_bytes(b: &[u8]) -> Self {
        b[..N].try_into().unwrap()
    }
    fn kbytes(v: &&Self) -> Vec<u8> {
        plain_bytes(*v)
    }
}
impl MxKey for i8 {
    fn from_bytes(b: &[u8]) -> Self {
        (b[0] ^ 0x80) as i8
    }
    fn kbytes(v: &&Self) -> Vec<u8> {
        plain_bytes(*v)
    }
}
impl MxKey for u16 {
    fn from_bytes(b: &[u8]) -> Self {
        u16::from_be_bytes([b[0], b[1]])
    }
    fn kbytes(v: &&Self) -> Vec<u8> {
        plain_bytes(*v)
    }
}
impl MxKey for i128 {
    fn from_bytes(b: &[u8]) -> Self {
        (u128::from_be_bytes(b[..16].try_into().unwrap()) ^ (1 << 127)) as i128
    }
    fn kbytes(v: &&Self) -> Vec<u8> {
        plain_bytes(*v)
    }
}

// ---------------------------------------------------------------- values

/// A value type: made from a number and a length, compared by its bytes.
pub trait MxVal: Clone + Debug + Send + Sync + 'static + DiskType {
    fn make(n: u64, len: usize) -> Self;
    fn bytes(&self) -> Vec<u8>;
    fn vbytes(v: &Self::View<'_>) -> Vec<u8>;
}

macro_rules! plain_val {
    ($t:ty, $mk:expr) => {
        impl MxVal for $t {
            fn make(n: u64, len: usize) -> Self {
                ($mk)(n, len)
            }
            fn bytes(&self) -> Vec<u8> {
                plain_bytes(self)
            }
            fn vbytes(v: &&$t) -> Vec<u8> {
                plain_bytes(*v)
            }
        }
    };
}

plain_val!(u8, |n: u64, _| n as u8);
plain_val!(i16, |n: u64, _| n as i16);
plain_val!(u128, |n: u64, l: usize| ((n as u128) << 64) | (n.rotate_left(l as u32 % 64) as u128));
plain_val!(f64, |n: u64, _| f64::from_bits(n));
plain_val!(f32, |n: u64, _| f32::from_bits(n as u32));
plain_val!([u8; 0], |_, _| []);
plain_val!([u8; 5], |n: u64, _| { let b = n.to_le_bytes(); [b[0], b[1], b[2], b[3], b[4]] });
plain_val!([u8; 33], |n: u64, _| { let mut a = [0u8; 33]; for (i, x) in a.iter_mut().enumerate() { *x = (n >> (i % 8 * 8)) as u8 ^ i as u8; } a });

fn val_bytes(n: u64, len: usize) -> Vec<u8> {
    (0..len).map(|i| (n.rotate_left(i as u32 % 64) as u8) ^ (i as u8)).collect()
}

impl MxVal for Vec<u8> {
    fn make(n: u64, len: usize) -> Self {
        val_bytes(n, len)
    }
    fn bytes(&self) -> Vec<u8> {
        self.clone()
    }
    fn vbytes(v: &&[u8]) -> Vec<u8> {
        v.to_vec()
    }
}
impl MxVal for String {
    fn make(n: u64, len: usize) -> Self {
        let chars = ['a', 'é', '\0', '中', '\u{10FFFF}', 'z'];
        (0..len).map(|i| chars[((n >> (i % 16 * 4)) as usize + i) % chars.len()]).collect()
    }
    fn bytes(&self) -> Vec<u8> {
        self.as_bytes().to_vec()
    }
    fn vbytes(v: &&str) -> Vec<u8> {
        v.as_bytes().to_vec()
    }
}

// ---------------------------------------------------------------- cases

/// A key: from the case's shared prefix (or not), then `tail` at `at` bytes
/// past it, zeros elsewhere.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RawKey {
    pub shared: bool,
    pub at: u8,
    pub tail: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MxProbe {
    Get(RawKey),
    GetMany(Vec<RawKey>),
    Range(Option<(bool, RawKey)>, Option<(bool, RawKey)>, Vec<bool>),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MxCase {
    /// Bytes every shared key starts with.
    pub prefix: Vec<u8>,
    /// Keys, value numbers and value lengths.
    pub entries: Vec<(RawKey, u64, u16)>,
    pub probes: Vec<MxProbe>,
    /// 0 std map, 1 unsync map, 2 sync map (`iter_mut`, by its owner), 3
    /// owned entries, 4 `&mut` entries, 5 `Cow` / borrowed views.
    pub input: u8,
    pub offset: u16,
    pub huge: bool,
    /// Entries out of order: (position, 0 swap with the next, 1 repeat, 2
    /// reverse all from there).
    pub disorder: Option<(u16, u8)>,
}

fn raw_key() -> BoxGen<RawKey> {
    let byte = gens::pick_from(vec![0u8, 1, 2, b'a', 0x7f, 0x80, 0xfe, 0xff]);
    gens::frequency(vec![
        (6, (gens::bool(), gens::int(0u8..=3), gens::vec(byte.clone()).len(0..=3)).boxed()),
        (2, (gens::bool(), gens::int(0u8..=80), gens::vec(byte.clone()).len(0..=8)).boxed()),
        (1, (gens::bool(), gens::int(0u8..=4), gens::vec(gens::int(0u8..=255)).len(0..=24)).boxed()),
    ])
    .map(|(shared, at, tail)| RawKey { shared, at, tail })
    .boxed()
}

fn bound() -> BoxGen<Option<(bool, RawKey)>> {
    gens::frequency(vec![(1, gens::constant(None).boxed()), (4, (gens::bool(), raw_key()).map(Some).boxed())]).boxed()
}

pub fn gen_case() -> BoxGen<MxCase> {
    let prefix = gens::frequency(vec![
        (2, gens::constant(Vec::new()).boxed()),
        (3, gens::vec(gens::pick_from(vec![0u8, 7, b'k', 0xff])).len(1..=6).boxed()),
        (1, gens::vec(gens::int(0u8..=255)).len(8..=72).boxed()),
    ]);
    let probe = gens::frequency(vec![
        (3, raw_key().map(MxProbe::Get).boxed()),
        (1, gens::vec(raw_key()).len(1..=4).map(MxProbe::GetMany).boxed()),
        (3, (bound(), bound(), gens::vec(gens::bool()).len(0..=24)).map(|(s, e, w)| MxProbe::Range(s, e, w)).boxed()),
    ]);
    let entry = (raw_key(), gens::int(0u64..=u64::MAX), gens::frequency(vec![(5, gens::int(0u16..=12).boxed()), (1, gens::int(0u16..=3000).boxed())]));
    let size = gens::frequency(vec![(3, gens::vec(entry.clone()).len(0..=40).boxed()), (2, gens::vec(entry.clone()).len(0..=600).boxed()), (1, gens::vec(entry).len(0..=3000).boxed())]);
    let disorder = gens::frequency(vec![(5, gens::constant(None).boxed()), (1, (gens::int(0u16..=3000), gens::int(0u8..=2)).map(Some).boxed())]);
    (prefix, size, gens::vec(probe.boxed()).len(0..=12), gens::int(0u8..=5), gens::int(0u16..=5000), gens::bool(), disorder)
        .map(|(prefix, entries, probes, input, offset, huge, disorder)| MxCase { prefix, entries, probes, input, offset, huge, disorder })
        .boxed()
}

/// The key's encoding, 128 bytes, zero past what it sets.
fn key_bytes(prefix: &[u8], raw: &RawKey) -> [u8; 128] {
    let mut b = [0u8; 128];
    let mut at = 0;
    if raw.shared {
        b[..prefix.len()].copy_from_slice(prefix);
        at = prefix.len();
    }
    at += raw.at as usize;
    for (i, &t) in raw.tail.iter().enumerate() {
        if at + i < 128 {
            b[at + i] = t;
        }
    }
    b
}

fn mk<K: MxKey>(prefix: &[u8], raw: &RawKey) -> K {
    K::from_bytes(&key_bytes(prefix, raw))
}

fn hex(b: &[u8]) -> String {
    let s: String = b.iter().take(48).map(|x| format!("{x:02x}")).collect();
    if b.len() > 48 { format!("{s}..({} B)", b.len()) } else { s }
}

type Ent = (Vec<u8>, Vec<u8>);

fn ent<K: MxKey, V: MxVal>(k: &K, v: &V) -> Ent {
    (plain_bytes(k), v.bytes())
}

fn show(e: &Option<Ent>) -> String {
    match e {
        None => "None".into(),
        Some((k, v)) => format!("({}, {})", hex(k), hex(v)),
    }
}

/// The entries in the order the case hands them to a writer, with its
/// disorder applied; whether they ascend strictly, and if not the index of
/// the first that doesn't.
fn ordered<K: MxKey, V: MxVal>(model: &BTreeMap<K, V>, disorder: Option<(u16, u8)>) -> (Vec<(K, V)>, Option<usize>) {
    let mut v: Vec<(K, V)> = model.iter().map(|(k, v)| (*k, v.clone())).collect();
    if let Some((pos, kind)) = disorder
        && v.len() >= 2
    {
        let p = pos as usize % (v.len() - 1);
        match kind {
            0 => v.swap(p, p + 1),
            1 => {
                let e = v[p].clone();
                v.insert(p + 1, e);
            }
            _ => v[p..].reverse(),
        }
    }
    let first_bad = (1..v.len()).find(|&i| v[i - 1].0 >= v[i].0);
    (v, first_bad)
}

/// A value as a writer's input other than `&V`: `Cow` bytes / `&str` for
/// text and bytes, the value itself for plain ones.
pub trait AsInput {
    type In<'a>
    where
        Self: 'a;
    fn as_input(&self) -> Self::In<'_>;
}
impl AsInput for Vec<u8> {
    type In<'a> = Cow<'a, [u8]>;
    fn as_input(&self) -> Cow<'_, [u8]> {
        if self.len() % 2 == 0 { Cow::Borrowed(self) } else { Cow::Owned(self.clone()) }
    }
}
impl AsInput for String {
    type In<'a> = Cow<'a, str>;
    fn as_input(&self) -> Cow<'_, str> {
        if self.len() % 2 == 0 { Cow::Borrowed(self) } else { Cow::Owned(self.clone()) }
    }
}
macro_rules! as_input_plain {
    ($($t:ty),*) => {$(
        impl AsInput for $t {
            type In<'a> = $t;
            fn as_input(&self) -> $t {
                *self
            }
        }
    )*};
}
as_input_plain!(u8, i16, u128, f64, f32, [u8; 0], [u8; 5], [u8; 33]);

// ---------------------------------------------------------------- writing

enum How<'f> {
    Create(&'f std::path::Path),
    WriteAt(&'f mut File, u64),
}

/// Writes `entries` by `input` (see `MxCase::input`): `create` or
/// `write_at`, generic over the entries' input types, called per path.
fn write_entries<K, V>(how: How<'_>, input: u8, entries: &[(K, V)]) -> std::io::Result<Option<std::ops::Range<u64>>>
where
    K: MxKey + DiskInput<K>,
    V: MxVal + AsInput + DiskInput<V::Stored>,
    for<'a> V::In<'a>: DiskInput<V::Stored>,
    <K as DiskType>::Strategy: InlineKeyStrategy<K>,
    K: Storable<InWriter = K>,
    <K as DiskType>::Strategy: Default,
{
    macro_rules! go {
        ($it:expr) => {
            match how {
                How::Create(p) => BBTreeMapFile::<K, V>::create(p, $it).map(|_| None),
                How::WriteAt(f, off) => BBTreeMapFile::<K, V>::write_at(f, off, $it).map(Some),
            }
        };
    }
    match input {
        1 => {
            let mut map = bbtree::unsync::BBTreeMap::<K, V>::new();
            for (k, v) in entries {
                map.insert(*k, v.clone());
            }
            go!(&map)
        }
        2 => {
            let mut map = bbtree::BBTreeMap::<K, V, K::Strategy>::with_strategy(Default::default());
            for (k, v) in entries {
                let _ = map.insert(*k, v.clone());
            }
            go!(map.iter_mut())
        }
        3 => go!(entries.iter().cloned()),
        4 => {
            let mut owned = entries.to_vec();
            go!(owned.iter_mut().map(|(k, v)| (k, v)))
        }
        5 => go!(entries.iter().map(|(k, v)| (*k, v.as_input()))),
        _ => go!(entries.iter().map(|(k, v)| (k, v))),
    }
}

// ---------------------------------------------------------------- property

/// The case against std's map, through every way of opening the file.
/// `sync`: the key type has inline keys, so the sync map can hold it.
pub fn run<K, V, const SYNC: bool>(case: &MxCase) -> Result<(), Fail>
where
    K: MxKey + DiskInput<K> + Storable<InWriter = K>,
    V: MxVal + AsInput + DiskInput<V::Stored>,
    for<'a> V::In<'a>: DiskInput<V::Stored>,
    <K as DiskType>::Strategy: InlineKeyStrategy<K> + Default,
{
    let _ = take_premise();
    let r = run_inner::<K, V, SYNC>(case);
    if let Some(what) = take_premise() {
        let area = if what.starts_with("hash_tag") {
            "hash_tag"
        } else if what.starts_with("cmp_from called") {
            "cmp_from_called"
        } else if what.starts_with("cmp_prefix") {
            "cmp_prefix_len"
        } else {
            "cmp_from"
        };
        return Err(fail(&format!("mmapmx__contract__{area}"), what));
    }
    r
}

fn run_inner<K, V, const SYNC: bool>(case: &MxCase) -> Result<(), Fail>
where
    K: MxKey + DiskInput<K> + Storable<InWriter = K>,
    V: MxVal + AsInput + DiskInput<V::Stored>,
    for<'a> V::In<'a>: DiskInput<V::Stored>,
    <K as DiskType>::Strategy: InlineKeyStrategy<K> + Default,
{
    let p = &case.prefix;
    let mut model: BTreeMap<K, V> = BTreeMap::new();
    for (raw, n, len) in &case.entries {
        model.insert(mk::<K>(p, raw), V::make(*n, *len as usize));
    }
    let (entries, first_bad) = ordered(&model, case.disorder);
    let mut input = case.input;
    if (input == 2 && !SYNC) || (first_bad.is_some() && (input == 1 || input == 2)) {
        // the maps put entries in order: hand the disordered ones over as they are
        input = 0;
    }

    if let Some(bad) = first_bad {
        classify(true, "disordered");
        // create: InvalidInput, before the file is touched
        let path = TempPath::new();
        let before: Vec<u8> = (0..(case.offset as usize % 300)).map(|i| i as u8 ^ 0x5a).collect();
        fs::write(&path.0, &before).unwrap();
        let r = write_entries::<K, V>(How::Create(&path.0), input, &entries);
        ensure(matches!(&r, Err(e) if e.kind() == std::io::ErrorKind::InvalidInput), "mmapmx__accepted__create_disordered", || {
            format!("create of {} entries, entry {bad} not above the one before (input {input}): {:?}", entries.len(), r.as_ref().map(|_| ()))
        })?;
        ensure(fs::read(&path.0).unwrap() == before, "mmapmx__touched__create_disordered", || {
            format!("create refused disordered entries but changed the file ({} bytes before, {} after)", before.len(), fs::read(&path.0).unwrap().len())
        })?;
        // write_at: "file is untouched if they don't ascend"
        let mut file = OpenOptions::new().read(true).write(true).open(&path.0).unwrap();
        let r = write_entries::<K, V>(How::WriteAt(&mut file, case.offset as u64 % 400), input, &entries);
        drop(file);
        ensure(matches!(&r, Err(e) if e.kind() == std::io::ErrorKind::InvalidInput), "mmapmx__accepted__write_at_disordered", || {
            format!("write_at of {} entries, entry {bad} not above the one before (input {input}): {:?}", entries.len(), r.as_ref().map(|_| ()))
        })?;
        ensure(fs::read(&path.0).unwrap() == before, "mmapmx__touched__write_at_disordered", || {
            format!("write_at refused disordered entries but changed the file ({} bytes before, {} after)", before.len(), fs::read(&path.0).unwrap().len())
        })?;
        return Ok(());
    }

    let whole = TempPath::new();
    write_entries::<K, V>(How::Create(&whole.0), input, &entries)
        .map_err(|e| fail("mmapmx__error__create", format!("create of {} entries (input {input}): {e}", entries.len())))?;
    let offset = case.offset as u64;
    let embedded = TempPath::new();
    let range = {
        let mut file = OpenOptions::new().read(true).write(true).truncate(true).open(&embedded.0).unwrap();
        file.write_all(&vec![0xC3; offset as usize + 77]).unwrap();
        write_entries::<K, V>(How::WriteAt(&mut file, offset), input, &entries)
            .map_err(|e| fail("mmapmx__error__write_at", format!("write_at {offset} (input {input}): {e}")))?
            .unwrap()
    };
    let embedded_bytes = fs::read(&embedded.0).unwrap();
    ensure(
        range.start >= offset && range.end == embedded_bytes.len() as u64 && embedded_bytes[..offset as usize].iter().all(|&b| b == 0xC3),
        "mmapmx__range__write_at",
        || format!("write_at({offset}) returned {range:?}, file of {} bytes after", embedded_bytes.len()),
    )?;
    let whole_bytes = fs::read(&whole.0).unwrap();
    let pages = if case.huge { OsPages::Huge } else { OsPages::Small };
    let opened: Vec<(&str, std::io::Result<BBTreeMapFile<K, V>>)> = vec![
        ("read", BBTreeMapFile::<K, V>::read(&whole.0)),
        ("mmap", unsafe { BBTreeMapFile::<K, V>::mmap(&whole.0).pages(pages).open() }),
        ("read_range", BBTreeMapFile::<K, V>::read_range(&mut File::open(&embedded.0).unwrap(), range.clone())),
        ("mmap_range", unsafe { BBTreeMapFile::<K, V>::mmap(&embedded.0).range(range.clone()).pages(pages).open() }),
    ];
    let want: Vec<Ent> = model.iter().map(|(k, v)| ent(k, v)).collect();
    for (how, file) in opened {
        let file = file.map_err(|e| fail(&format!("mmapmx__open_failed__{how}"), format!("{how}: {e}")))?;
        let fe = |(k, v): (K::View<'_>, V::View<'_>)| -> Ent { (K::kbytes(&k), V::vbytes(&v)) };
        ensure(file.len() == model.len() && file.is_empty() == model.is_empty(), "mmapmx__mismatch__len", || {
            format!("{how}: len {} vs {}", file.len(), model.len())
        })?;
        let got: Vec<Ent> = file.iter().map(fe).collect();
        if got != want {
            let at = got.iter().zip(&want).position(|(a, b)| a != b);
            return Err(fail("mmapmx__mismatch__iter", format!(
                "{how}: iter {} entries vs {}; first difference at {at:?}: {} vs {}",
                got.len(), want.len(), show(&at.map(|i| got[i].clone())), show(&at.map(|i| want[i].clone()))
            )));
        }
        let rev: Vec<Ent> = file.iter().rev().map(fe).collect();
        ensure(rev.iter().rev().eq(want.iter()), "mmapmx__mismatch__iter_rev", || format!("{how}: iter().rev() differs"))?;
        for (k, v) in model.iter().step_by((model.len() / 48).max(1)) {
            let got = file.get(k).map(|v| V::vbytes(&v));
            ensure(got.as_ref() == Some(&v.bytes()), "mmapmx__mismatch__get_stored", || {
                format!("{how}: get({k:?}) {:?} vs {}", got.map(|b| hex(&b)), hex(&v.bytes()))
            })?;
        }
        for probe in &case.probes {
            match probe {
                MxProbe::Get(raw) => {
                    let k: K = mk(p, raw);
                    let got = file.get(&k).map(|v| V::vbytes(&v));
                    let exp = model.get(&k).map(|v| v.bytes());
                    ensure(got == exp, "mmapmx__mismatch__get", || format!("{how}: get({k:?}) {:?} vs {:?}", got.map(|b| hex(&b)), exp.map(|b| hex(&b))))?;
                    let got = file.get_key_value(&k).map(fe);
                    let exp = model.get_key_value(&k).map(|(k, v)| ent(k, v));
                    ensure(got == exp, "mmapmx__mismatch__get_key_value", || format!("{how}: get_key_value({k:?}) {} vs {}", show(&got), show(&exp)))?;
                }
                MxProbe::GetMany(raws) => {
                    let ks: Vec<K> = raws.iter().map(|r| mk(p, r)).collect();
                    let exp: Vec<Option<Vec<u8>>> = ks.iter().map(|k| model.get(k).map(|v| v.bytes())).collect();
                    let got: Vec<Option<Vec<u8>>> = match ks.len() {
                        1 => file.get_many([&ks[0]]).iter().map(|v| v.as_ref().map(V::vbytes)).collect(),
                        2 => file.get_many([&ks[0], &ks[1]]).iter().map(|v| v.as_ref().map(V::vbytes)).collect(),
                        3 => file.get_many([&ks[0], &ks[1], &ks[2]]).iter().map(|v| v.as_ref().map(V::vbytes)).collect(),
                        _ => file.get_many([&ks[0], &ks[1], &ks[2], &ks[3]]).iter().map(|v| v.as_ref().map(V::vbytes)).collect(),
                    };
                    ensure(got == exp, "mmapmx__mismatch__get_many", || format!("{how}: get_many({ks:?}): {got:?} vs {exp:?}"))?;
                }
                MxProbe::Range(s, e, walk) => {
                    let b = |x: &Option<(bool, RawKey)>| -> Bound<K> {
                        match x {
                            None => Bound::Unbounded,
                            Some((true, r)) => Bound::Included(mk(p, r)),
                            Some((false, r)) => Bound::Excluded(mk(p, r)),
                        }
                    };
                    let (s, e) = (b(s), b(e));
                    if range_panics(&s, &e) {
                        let ours = catch_unwind(AssertUnwindSafe(|| file.range::<K, _>((s, e)).count())).is_err();
                        ensure(ours || model.is_empty(), "mmapmx__panic_mismatch__range", || format!("{how}: range({s:?}, {e:?}) did not panic, std's does"))?;
                        continue;
                    }
                    compare_walk(file.range::<K, _>((s, e)).map(fe), model.range((s, e)).map(|(k, v)| ent(k, v)), walk)
                        .map_err(|why| fail("mmapmx__mismatch__range", format!("{how}: range({s:?}, {e:?}): {why}")))?;
                }
            }
        }
    }
    // The same entries written again, with the writer's arena elsewhere,
    // make the same file: a file map holds its entries' bytes and spans,
    // "zeros, not stale pointers" (tree/file.rs, write_leaf_bytes), so
    // nothing in it depends on where the writer's tree or arena lay.
    let shift: Vec<Vec<u8>> = (0..1 + entries.len() % 7).map(|i| vec![i as u8; 1 << (8 + i)]).collect();
    let again = TempPath::new();
    write_entries::<K, V>(How::Create(&again.0), input, &entries)
        .map_err(|e| fail("mmapmx__error__create", format!("second create (input {input}): {e}")))?;
    drop(shift);
    let again = fs::read(&again.0).unwrap();
    if again != whole_bytes {
        let diffs: Vec<usize> = again.iter().zip(&whole_bytes).enumerate().filter(|(_, (a, b))| a != b).map(|(i, _)| i).collect();
        let words: Vec<String> = diffs.iter().map(|i| i & !7).fold(Vec::new(), |mut v: Vec<usize>, w| {
            if v.last() != Some(&w) { v.push(w); }
            v
        }).iter().take(6).map(|&w| format!("@{w}: {} vs {}", hex(&whole_bytes[w..(w + 8).min(whole_bytes.len())]), hex(&again[w..(w + 8).min(again.len())]))).collect();
        return Err(fail("mmapmx__nondeterministic__create", format!(
            "two creates of the same {} entries (input {input}): {} vs {} bytes, {} differ; words {}",
            entries.len(), whole_bytes.len(), again.len(), diffs.len(), words.join(", ")
        )));
    }
    // the embedded copy is the same map as the whole file
    ensure(embedded_bytes[range.start as usize..] == whole_bytes[..], "mmapmx__mismatch__write_at_bytes", || {
        format!("write_at({offset}) at {range:?} wrote {} bytes, create {}", range.end - range.start, whole_bytes.len())
    })?;

    classify(model.len() >= 1000, "mmapmx_1k");
    Ok(())
}

/// For plain values: `bbtree::file`'s writer and reader of the same entries.
pub fn run_file<K, V, const SYNC: bool>(case: &MxCase) -> Result<(), Fail>
where
    K: MxKey + DiskInput<K> + Storable<InWriter = K>,
    V: MxVal + AsInput + DiskInput<V::Stored> + PlainBytes + Storable<InWriter = V> + Copy,
    for<'a> V::In<'a>: DiskInput<V::Stored>,
    <K as DiskType>::Strategy: InlineKeyStrategy<K> + Default,
{
    run::<K, V, SYNC>(case)?;
    let _ = take_premise();
    let r = file_inner::<K, V, SYNC>(case);
    if let Some(what) = take_premise() {
        return Err(fail("mmapmx__contract__file", what));
    }
    r
}

fn file_inner<K, V, const SYNC: bool>(case: &MxCase) -> Result<(), Fail>
where
    K: MxKey + Storable<InWriter = K>,
    V: MxVal + PlainBytes + Storable<InWriter = V> + Copy,
    <K as DiskType>::Strategy: InlineKeyStrategy<K> + Default,
{
    type S<K> = <K as DiskType>::Strategy;
    let p = &case.prefix;
    let mut model: BTreeMap<K, V> = BTreeMap::new();
    for (raw, n, len) in &case.entries {
        model.insert(mk::<K>(p, raw), V::make(*n, *len as usize));
    }
    let (entries, first_bad) = ordered(&model, case.disorder);
    let asc = FileWriter::<K, V, S<K>>::from_ascending(entries.iter().copied());
    if let Some(bad) = first_bad {
        return match asc {
            Ok(_) => Err(fail("mmapmx__accepted__from_ascending", format!("{} entries, {bad} not above the one before", entries.len()))),
            Err(e) => ensure(e.index() == bad, "mmapmx__index__from_ascending", || format!("NotAscending at {}, first bad entry {bad}", e.index())),
        };
    }
    let asc = asc.map_err(|e| fail("mmapmx__refused__from_ascending", format!("ascending entries refused: {e}")))?;
    let buf = asc.to_buf();
    let mut unsync = bbtree::unsync::BBTreeMap::<K, V, S<K>>::with_strategy(Default::default());
    unsync.extend(entries.iter().copied());
    ensure(FileWriter::from(&unsync).to_buf()[..] == buf[..], "mmapmx__mismatch__from_unsync", || "FileWriter::from(&unsync) and from_ascending differ".into())?;
    if SYNC {
        let mut sync = bbtree::BBTreeMap::<K, V, S<K>>::with_strategy(Default::default());
        for (k, v) in &entries {
            let _ = sync.insert(*k, *v);
        }
        let w = FileWriter::from(&mut sync);
        ensure(w.to_buf()[..] == buf[..] && w.len() == model.len(), "mmapmx__mismatch__from_sync", || {
            format!("FileWriter::from(&mut sync): {} entries, {} bytes; from_ascending {} bytes", w.len(), w.byte_len(), buf.len())
        })?;
    }
    let file = FileMap::<K, V, S<K>>::open(&buf).map_err(|e| fail("mmapmx__open_refused__file", format!("{e}")))?;
    let pk = |k: &K| plain_bytes(k);
    let pv = |v: &V| plain_bytes(v);
    ensure(file.keys().map(pk).eq(model.keys().map(pk)) && file.values().map(pv).eq(model.values().map(pv)), "mmapmx__mismatch__keys_values", || {
        "FileMap keys()/values() differ from std's".into()
    })?;
    ensure(file.keys().rev().map(pk).eq(model.keys().rev().map(pk)) && file.values().len() == model.len(), "mmapmx__mismatch__keys_values", || {
        "FileMap keys().rev() / values().len() differ from std's".into()
    })?;
    if model.len() <= 200 {
        let (a, b) = (format!("{file:?}"), format!("{model:?}"));
        ensure(a == b, "mmapmx__mismatch__debug", || format!("Debug {a} vs {b}"))?;
    }
    for probe in &case.probes {
        if let MxProbe::Range(s, e, walk) = probe {
            let b = |x: &Option<(bool, RawKey)>| -> Bound<K> {
                match x {
                    None => Bound::Unbounded,
                    Some((true, r)) => Bound::Included(mk(p, r)),
                    Some((false, r)) => Bound::Excluded(mk(p, r)),
                }
            };
            let (s, e) = (b(s), b(e));
            if range_panics(&s, &e) {
                continue;
            }
            let mut ours = file.range((s, e));
            let mut std = model.range((s, e));
            let half = walk.len() / 2;
            for &front in &walk[..half] {
                let (a, b) = if front { (ours.next(), std.next()) } else { (ours.next_back(), std.next_back()) };
                ensure(a.map(|(k, v)| (pk(k), pv(v))) == b.map(|(k, v)| (pk(k), pv(v))), "mmapmx__mismatch__file_range", || format!("range({s:?}, {e:?}) step differs"))?;
            }
            // a clone goes on from where the original is, apart from it
            let cl = ours.clone();
            compare_walk(cl.map(|(k, v)| (pk(k), pv(v))), std.clone().map(|(k, v)| (pk(k), pv(v))), &walk[half..])
                .map_err(|why| fail("mmapmx__mismatch__range_clone", format!("range({s:?}, {e:?}) clone: {why}")))?;
            let got = ours.rfold(Vec::new(), |mut acc, (k, v)| {
                acc.push((pk(k), pv(v)));
                acc
            });
            let exp = std.rfold(Vec::new(), |mut acc, (k, v)| {
                acc.push((pk(k), pv(v)));
                acc
            });
            ensure(got == exp, "mmapmx__mismatch__range_rfold", || format!("range({s:?}, {e:?}) rfold: {} vs {} entries", got.len(), exp.len()))?;
        }
    }
    Ok(())
}
