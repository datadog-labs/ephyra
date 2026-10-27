// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Shared by the properties: the failure type (a stable signature and a
//! message), generators of keys spread so that maps of every height
//! appear, range bounds that hit and miss stored keys, byte-level damage
//! for the readers that promise to stay memory-safe, and walk comparisons.

use std::ops::Bound;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::OnceLock;

use ephyra::{BoxGen, Gen, GenExt, gens};
use serde::{Deserialize, Serialize};

/// A failed check: `sig` is `<oracle>__<kind>__<area>`, never with values
/// in it; `msg` has the values.
#[derive(Clone, Debug)]
pub struct Fail {
    pub sig: String,
    pub msg: String,
}

pub fn fail(sig: &str, msg: impl Into<String>) -> Fail {
    Fail { sig: sig.to_string(), msg: msg.into() }
}

/// `Err(fail(sig, msg))` when `bad`.
pub fn ensure(ok: bool, sig: &str, msg: impl FnOnce() -> String) -> Result<(), Fail> {
    if ok { Ok(()) } else { Err(fail(sig, msg())) }
}

/// Runs `f`, turning a panic into `None`: for the reads of a damaged map,
/// where the contract allows a panic.
pub fn quietly<T>(f: impl FnOnce() -> T) -> Option<T> {
    catch_unwind(AssertUnwindSafe(f)).ok()
}

/// Self-test switches (README): off unless the variable is set to 1.
pub fn selftest(name: &str) -> bool {
    static BREAK: OnceLock<bool> = OnceLock::new();
    static ABORT: OnceLock<bool> = OnceLock::new();
    let cell = match name {
        "break" => &BREAK,
        "abort" => &ABORT,
        _ => return false,
    };
    *cell.get_or_init(|| std::env::var(format!("BBZ_SELFTEST_{}", name.to_uppercase())).as_deref() == Ok("1"))
}

/// A key as a raw number, mapped to each key type by [`FromRaw`]: a mix of
/// a dense range (collisions, adjacent keys), a wider one (many leaves) and
/// the edges of the whole range (sign bits, inline-key prefixes).
pub fn raw_key() -> BoxGen<u128> {
    gens::frequency(vec![
        (4, gens::int(0u64..64).map(u128::from).boxed()),
        (4, gens::int(0u64..20_000).map(u128::from).boxed()),
        (1, wide(gens::edges(0u64..=u64::MAX)).boxed()),
        (1, wide(gens::int(0u64..=u64::MAX)).boxed()),
    ])
    .boxed()
}

/// A u128 of two halves drawn from `half`, so that both halves see edges.
fn wide(half: impl Gen<Value = u64> + Clone + 'static) -> BoxGen<u128> {
    (half.clone(), half)
        .iso(|(hi, lo)| (u128::from(hi) << 64) | u128::from(lo), |w: &u128| ((w >> 64) as u64, *w as u64))
        .boxed()
}

/// How many entries a map gets: mostly small, sometimes enough for a tree
/// of three or four levels.
pub fn map_len() -> BoxGen<usize> {
    gens::frequency(vec![
        (3, gens::int(0usize..=16).boxed()),
        (3, gens::int(0usize..=400).boxed()),
        (2, gens::int(0usize..=6_000).boxed()),
        (1, gens::int(0usize..=40_000).boxed()),
    ])
    .boxed()
}

/// Entries whose keys come from a seed: `len` keys stepping from `start` by
/// `stride`, plus `extra` scattered ones; values are arbitrary. A seed
/// stays small to print and shrink while the map it describes is large.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MapSeed {
    pub start: u128,
    pub stride: u128,
    pub len: usize,
    pub extra: Vec<u128>,
    pub value_salt: u64,
}

impl MapSeed {
    pub fn generator() -> BoxGen<MapSeed> {
        (
            raw_key(),
            gens::frequency(vec![
                (4, gens::int(1u64..=3).map(u128::from).boxed()),
                (2, gens::int(1u64..=1_000).map(u128::from).boxed()),
                (1, wide(gens::edges(0u64..=u64::MAX)).boxed()),
            ]),
            map_len(),
            gens::vec(raw_key()).len(0..=24),
            gens::int(0u64..=u64::MAX),
        )
            .iso(
                |(start, stride, len, extra, value_salt)| MapSeed { start, stride, len, extra, value_salt },
                |s: &MapSeed| (s.start, s.stride, s.len, s.extra.clone(), s.value_salt),
            )
            .boxed()
    }

    /// The raw keys, unsorted and possibly repeated (wrapping arithmetic).
    pub fn raw_keys(&self) -> impl Iterator<Item = u128> + '_ {
        (0..self.len as u128)
            .map(|i| self.start.wrapping_add(i.wrapping_mul(self.stride)))
            .chain(self.extra.iter().copied())
    }

    pub fn value_of(&self, raw: u128) -> u64 {
        let mixed = (raw as u64) ^ ((raw >> 64) as u64).rotate_left(17) ^ self.value_salt;
        mixed.wrapping_mul(0x9E37_79B9_7F4A_7C15)
    }
}

/// Key and value types made from a raw number. Wrapping, so that every
/// type sees its own edges.
pub trait FromRaw: Copy + Ord + std::fmt::Debug {
    fn from_raw(raw: u128) -> Self;
}

macro_rules! from_raw_int {
    ($($t:ty),*) => {$(
        impl FromRaw for $t {
            fn from_raw(raw: u128) -> Self { raw as $t }
        }
    )*};
}
from_raw_int!(u8, u16, u32, u64, u128, i8, i16, i32, i64, i128);

impl<const N: usize> FromRaw for [u8; N] {
    /// Big-endian, so that the order of small raw numbers is the arrays'
    /// order, padded to N bytes (or truncated, keeping the low bytes).
    fn from_raw(raw: u128) -> Self {
        let be = raw.to_be_bytes();
        let mut out = [0u8; N];
        for (i, byte) in out.iter_mut().rev().enumerate() {
            *byte = if i < 16 { be[15 - i] } else { 0 };
        }
        out
    }
}

/// A range bound, raw.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum RawBound {
    Included(u128),
    Excluded(u128),
    Unbounded,
}

impl RawBound {
    pub fn generator() -> BoxGen<RawBound> {
        (gens::int(0u8..=4), raw_key())
            .iso(
                |(kind, raw)| match kind {
                    0 | 1 => RawBound::Included(raw),
                    2 | 3 => RawBound::Excluded(raw),
                    _ => RawBound::Unbounded,
                },
                |b: &RawBound| match *b {
                    RawBound::Included(raw) => (0, raw),
                    RawBound::Excluded(raw) => (2, raw),
                    RawBound::Unbounded => (4, 0),
                },
            )
            .boxed()
    }

    pub fn to<K: FromRaw>(self) -> Bound<K> {
        match self {
            RawBound::Included(raw) => Bound::Included(K::from_raw(raw)),
            RawBound::Excluded(raw) => Bound::Excluded(K::from_raw(raw)),
            RawBound::Unbounded => Bound::Unbounded,
        }
    }
}

/// Whether std's `BTreeMap::range` would panic on these bounds (start past
/// end, or equal and both excluded), if the map has a root: std checks the
/// bounds only then, so an empty map may go either way.
pub fn range_panics<K: Ord>(start: &Bound<K>, end: &Bound<K>) -> bool {
    match (start, end) {
        (Bound::Excluded(s), Bound::Excluded(e)) => s >= e,
        (Bound::Included(s) | Bound::Excluded(s), Bound::Included(e) | Bound::Excluded(e)) => s > e,
        _ => false,
    }
}

/// One way to damage a file map's bytes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Damage {
    /// XOR one byte.
    Flip { at: usize, mask: u8 },
    /// Overwrite a native-endian machine word with a value that is likely
    /// to mean something to a reader: an offset, a count, a length.
    Word { at: usize, value: u64 },
    /// Copy a run of bytes from one place to another (a page over another,
    /// an edge into a slot).
    Copy { from: usize, to: usize, len: usize },
    /// Cut the bytes short.
    Truncate { keep: usize },
}

impl Damage {
    /// Positions are raw numbers taken modulo the file's length when
    /// applied, so that a damage keeps its meaning as the map shrinks.
    pub fn generator() -> BoxGen<Damage> {
        let word = gens::frequency(vec![
            (3, gens::int(0u64..=64).boxed()),
            (2, gens::int(0u64..=1 << 20).boxed()),
            (2, gens::edges(0u64..=u64::MAX).boxed()),
            (1, gens::pick_from(vec![512u64, 1024, 2048, 4096, 8192, 1023, 1025, 4095]).boxed()),
        ]);
        let at = gens::int(0usize..=1 << 24);
        (gens::int(0u8..=9), at.clone(), at, gens::int(1u8..=255), word, gens::int(1usize..=256))
            .iso(
                |(kind, a, b, mask, value, len)| match kind {
                    0..=3 => Damage::Flip { at: a, mask },
                    4..=7 => Damage::Word { at: a, value },
                    8 => Damage::Copy { from: a, to: b, len },
                    _ => Damage::Truncate { keep: a },
                },
                |d: &Damage| match *d {
                    Damage::Flip { at, mask } => (0, at, 0, mask, 0, 1),
                    Damage::Word { at, value } => (4, at, 0, 1, value, 1),
                    Damage::Copy { from, to, len } => (8, from, to, 1, 0, len),
                    Damage::Truncate { keep } => (9, keep, 0, 1, 0, 1),
                },
            )
            .boxed()
    }

    /// Applies the damage; returns the new length of the valid prefix.
    /// Positions near the start (the header) and at word alignment are
    /// favoured by taking the raw position modulo a few page sizes.
    pub fn apply(&self, bytes: &mut [u8], len: usize) -> usize {
        if len == 0 {
            return 0;
        }
        let place = |raw: usize, width: usize| -> usize {
            let span = len.saturating_sub(width).max(1);
            match raw % 4 {
                0 => (raw / 4) % span.min(128),
                1 => ((raw / 4) % span) & !7,
                _ => (raw / 4) % span,
            }
        };
        match *self {
            Damage::Flip { at, mask } => {
                let at = place(at, 1);
                bytes[at] ^= mask;
                len
            }
            Damage::Word { at, value } => {
                if len < 8 {
                    return len;
                }
                let at = place(at, 8);
                bytes[at..at + 8].copy_from_slice(&value.to_ne_bytes());
                len
            }
            Damage::Copy { from, to, len: run } => {
                let run = run.min(len);
                let (from, to) = (place(from, run), place(to, run));
                bytes.copy_within(from..from + run, to);
                len
            }
            Damage::Truncate { keep } => keep % len,
        }
    }
}

/// Follows `walk` (`true` takes from the front, `false` from the back) over
/// both iterators, then drains the rest from the front; the first
/// difference, if any.
pub fn compare_walk<T: PartialEq + std::fmt::Debug>(
    mut ours: impl DoubleEndedIterator<Item = T>,
    mut model: impl DoubleEndedIterator<Item = T>,
    walk: &[bool],
) -> Result<(), String> {
    for (step, &front) in walk.iter().enumerate() {
        let (a, b) = if front { (ours.next(), model.next()) } else { (ours.next_back(), model.next_back()) };
        if a != b {
            return Err(format!("step {step} ({}): ours {a:?}, std {b:?}", if front { "next" } else { "next_back" }));
        }
    }
    let rest_ours: Vec<T> = ours.collect();
    let rest_model: Vec<T> = model.collect();
    if rest_ours != rest_model {
        let at = rest_ours.iter().zip(&rest_model).position(|(a, b)| a != b);
        return Err(format!(
            "after the walk: ours {} entries, std {}; first difference at {at:?}: {:?} vs {:?}",
            rest_ours.len(),
            rest_model.len(),
            at.map(|i| &rest_ours[i]),
            at.map(|i| &rest_model[i]),
        ));
    }
    Ok(())
}
