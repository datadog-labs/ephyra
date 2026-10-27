// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Generators shared by the bbtree properties: keys spread so that maps
//! of every height appear, range bounds that hit and miss stored keys, and
//! byte-level damage for the readers that promise to stay memory-safe.

use std::ops::Bound;

use ephyra::{BoxGen, Gen, GenExt, gens};

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
#[derive(Clone, Debug, PartialEq)]
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
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RawBound {
    Included(u128),
    Excluded(u128),
    Unbounded,
}

impl RawBound {
    pub fn generator() -> BoxGen<RawBound> {
        (gens::int(0u8..=4), raw_key()).iso(
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

/// Whether std's `BTreeMap::range` would panic on these bounds: start past
/// end, or equal and both excluded.
pub fn range_panics<K: Ord>(start: &Bound<K>, end: &Bound<K>) -> bool {
    match (start, end) {
        (Bound::Excluded(s), Bound::Excluded(e)) => s >= e,
        (Bound::Included(s) | Bound::Excluded(s), Bound::Included(e) | Bound::Excluded(e)) => s > e,
        _ => false,
    }
}

/// One way to damage a file map's bytes.
#[derive(Clone, Debug, PartialEq)]
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
        (gens::int(0u8..=9), at.clone(), at, gens::int(1u8..=255), word, gens::int(1usize..=256)).iso(
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

/// Aborts the process, printing the input, if one test case runs longer
/// than its deadline: ephyra's `timeout` bounds the whole run, and a hang
/// in a damaged map's walk is a finding.
pub struct Watchdog {
    state: std::sync::Arc<std::sync::Mutex<(u64, Option<(std::time::Instant, String)>)>>,
}

impl Watchdog {
    pub fn new(name: &'static str, deadline: std::time::Duration) -> Self {
        let state = std::sync::Arc::new(std::sync::Mutex::new((0u64, None::<(std::time::Instant, String)>)));
        let watched = std::sync::Arc::downgrade(&state);
        std::thread::spawn(move || {
            while let Some(state) = watched.upgrade() {
                if let Some((started, input)) = &state.lock().unwrap_or_else(|e| e.into_inner()).1 {
                    if started.elapsed() > deadline {
                        eprintln!("{name}: HANG, a test case ran past {deadline:?}\n  input: {input}");
                        std::process::abort();
                    }
                }
                drop(state);
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
        });
        Self { state }
    }

    /// Runs one test case under the deadline.
    pub fn run<T>(&self, input: &impl std::fmt::Debug, case: impl FnOnce() -> T) -> T {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).1 = Some((std::time::Instant::now(), format!("{input:?}")));
        let out = case();
        self.state.lock().unwrap_or_else(|e| e.into_inner()).1 = None;
        out
    }
}
