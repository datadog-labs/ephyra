// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! User `InlineKey` impls under `WithInlineKey`: the rows of the node plan
//! (`node_plan.rs`) no std key reaches. A key type of the user's picks its
//! width (2, 4 or 8 bytes in both node kinds, or 8 / 4 with `cmp_from`),
//! whether the leaves keep its hash tags, and whether the map compares ties
//! with its `cmp_from`; `cmp_prefix` is the trait's default or its own.
//!
//! `UK<W, F>` is a byte string (its bytes are its encoding, its `Ord` is
//! theirs, so the impl keeps the contract) whose impl is set by `W` (width)
//! and `F` (1: hash tag, 2: cmp_from, 4: own cmp_prefix). Its hash tag is a
//! hash of the bytes, not their length as std's is. Its `cmp_from` really
//! starts at `equal_len`, so a map that passes too large an `equal_len`
//! answers wrong; and it checks the premise the docs give it:
//!
//! - `cmp_from`: "given that the first `equal_len` bytes of their
//!   encodings, zero-padded, are equal"; "Called only if HAS_CMP_FROM".
//! - `hash_tag`: "Called only if HAS_HASH_TAG".
//! - `cmp_prefix`: "what all of its keys share, at most 64 bytes".
//!
//! A broken premise is recorded (a panic in these calls may abort, per the
//! docs) and reported after the case as `ukey__contract__<call>`. The keys
//! drive the existing runners (umodel, annex, synclin, bulk), which check
//! the maps against std; their signatures come back as `ukey__<kind>__<area>`.

use std::cmp::Ordering;
use std::fmt::{self, Debug};
use std::sync::Mutex;

use bbtree::{InlineKey, InlineWidth, InlineWord, WithInlineKey};

use crate::common::{Fail, fail};
use crate::{annex, bulk, synclin, syncthreads, umodel};

static BROKEN: Mutex<Option<(&'static str, String)>> = Mutex::new(None);

fn broken(call: &'static str, msg: impl FnOnce() -> String) {
    let mut b = BROKEN.lock().unwrap_or_else(|e| e.into_inner());
    if b.is_none() {
        *b = Some((call, msg()));
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UK<const W: u8, const F: u8>(pub Vec<u8>);

impl<const W: u8, const F: u8> Debug for UK<W, F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "UK({:?})", self.0.escape_ascii().to_string())
    }
}

fn byte(k: &[u8], i: usize) -> u8 {
    k.get(i).copied().unwrap_or(0)
}

impl<const W: u8, const F: u8> InlineKey for UK<W, F> {
    const WIDTH: InlineWidth = match W {
        2 => InlineWidth::Two,
        4 => InlineWidth::Four,
        _ => InlineWidth::Eight,
    };
    const HAS_HASH_TAG: bool = F & 1 != 0;
    const HAS_CMP_FROM: bool = F & 2 != 0;

    fn inline_key_at(&self, offset: usize) -> InlineWord {
        let mut w = [0u8; 8];
        for (i, b) in w.iter_mut().enumerate() {
            *b = offset.checked_add(i).map_or(0, |at| byte(&self.0, at));
        }
        InlineWord::from_be_bytes(w)
    }

    fn hash_tag(&self) -> u8 {
        if F & 1 == 0 {
            broken("hash_tag", || format!("hash_tag called on {self:?} though HAS_HASH_TAG is false"));
        }
        // FNV-1a, folded to a byte: equal keys share it.
        let h = self.0.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3));
        (h ^ h >> 8 ^ h >> 16 ^ h >> 32) as u8
    }

    fn cmp_from(&self, other: &Self, equal_len: usize) -> Ordering {
        if F & 2 == 0 {
            broken("cmp_from_called", || format!("cmp_from called on {self:?} though HAS_CMP_FROM is false"));
        }
        let (a, b) = (&self.0, &other.0);
        let upto = equal_len.min(a.len().max(b.len()));
        if let Some(i) = (0..upto).find(|&i| byte(a, i) != byte(b, i)) {
            broken("cmp_from", || format!("cmp_from({self:?}, {other:?}, equal_len {equal_len}): the encodings differ at byte {i}"));
        }
        let rest = |k: &[u8]| k.get(equal_len..).unwrap_or(&[]).to_vec();
        rest(a).cmp(&rest(b)).then(a.len().cmp(&b.len()))
    }

    fn cmp_prefix(&self, prefix: &[u8]) -> Ordering {
        if F & 4 == 0 {
            // The trait's default, kept reachable: compare a window at a time.
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
            let mut t = [0u8; 8];
            t[..tail.len()].copy_from_slice(tail);
            return window.cmp(&(InlineWord::from_be_bytes(t) >> dropped_bits));
        }
        if prefix.len() > 64 {
            broken("cmp_prefix_len", || format!("cmp_prefix({self:?}) with a {}-byte prefix (docs: at most 64)", prefix.len()));
        }
        (0..prefix.len()).map(|i| byte(&self.0, i).cmp(&prefix[i])).find(|o| o.is_ne()).unwrap_or(Ordering::Equal)
    }
}

// ---------- key shapes for each runner ----------

impl<const W: u8, const F: u8> umodel::UKey for UK<W, F> {
    /// 0: std's mixed byte strings (prefix comes and goes, 255+ bytes);
    /// 1: a u32 group then a string (three groups sharing 3 bytes);
    /// 2: keys that tie when zero-padded and differ in length only;
    /// 3: a 70-byte shared prefix (past the 64 bytes a search checks) on
    /// most keys, then a signed number, order-preserving.
    fn of(raw: u128, mode: u8) -> Self {
        UK(match mode % 4 {
            0 => umodel::bytes_of(raw, 3),
            1 => {
                let mut k = ((raw % 3) as u32).to_be_bytes().to_vec();
                k.extend(umodel::bytes_of(raw / 3, 0));
                k
            }
            2 => {
                let mut k = umodel::bytes_of(raw / 4, (raw / 4 % 2) as u8);
                k.resize(k.len() + (raw % 4) as usize, 0);
                k
            }
            _ => {
                let mut k = if raw % 5 == 0 { vec![] } else { vec![0x5A; 70] };
                k.extend(((raw as i64).wrapping_sub(500) as u64 ^ 1 << 63).to_be_bytes());
                k
            }
        })
    }
}

impl<const W: u8, const F: u8> annex::AKey for UK<W, F> {
    /// The number big-endian behind 2 bytes; every 7th key zero-padded to
    /// 260 bytes (saturated length, ties when padded), every 7th+2 to 40.
    fn of(n: u64) -> Self {
        let mut k = b"ak".to_vec();
        k.extend_from_slice(&n.to_be_bytes());
        match n % 7 {
            3 => k.resize(260, 0),
            5 => k.resize(40, b'z'),
            _ => {}
        }
        UK(k)
    }
    fn num(&self) -> u64 {
        self.0.get(2..10).map_or(u64::MAX, |b| u64::from_be_bytes(b.try_into().unwrap_or_default()))
    }
}

impl<const W: u8, const F: u8> syncthreads::Key for UK<W, F> {
    // As String's: under shrink the lowest numbers leave the shared prefix in
    // steps; every 3rd key gets trailing zeros (ties when padded).
    fn of(n: u32, shrink: bool) -> Self {
        let mut k = match (shrink, n) {
            (false, _) | (true, 32..) => format!("tenant/7/key/{n:07}"),
            (true, 0..8) => format!("t{n}"),
            (true, 8..16) => format!("tenant/{n:02}"),
            (true, 16..24) => format!("tenant/7/a{n:02}"),
            (true, _) => format!("tenant/7/key/!{n:02}"),
        }
        .into_bytes();
        if n % 3 == 1 {
            k.extend_from_slice(&[0, 0]);
        }
        UK(k)
    }
    fn number(&self, _: bool) -> u32 {
        let k: &[u8] = &self.0;
        let k = &k[..k.len() - k.iter().rev().take_while(|&&b| b == 0).count()];
        let digits = k.len() - k.iter().rev().take_while(|b| b.is_ascii_digit()).count();
        std::str::from_utf8(&k[digits..]).ok().and_then(|s| s.parse().ok()).unwrap_or(u32::MAX)
    }
    fn outside(n: u32) -> bool {
        n < 32
    }
}

impl<const W: u8, const F: u8> bulk::Key for UK<W, F> {
    /// A 20-byte shared prefix, the number big-endian, and 0-2 trailing zeros.
    fn of(n: u32) -> Self {
        let mut k = b"tenant/0042/userkey/".to_vec();
        k.extend_from_slice(&n.to_be_bytes());
        k.resize(k.len() + (n % 3) as usize, 0);
        UK(k)
    }
}

// ---------- runners ----------

fn wrap(run: impl FnOnce() -> Result<(), Fail>) -> Result<(), Fail> {
    wrap_as("ukey", run)
}

fn wrap_as(oracle: &str, run: impl FnOnce() -> Result<(), Fail>) -> Result<(), Fail> {
    *BROKEN.lock().unwrap_or_else(|e| e.into_inner()) = None;
    let r = run().map_err(|f| match f.sig.split_once("__") {
        Some((_, rest)) => fail(&format!("{oracle}__{rest}"), f.msg),
        None => f,
    });
    let b = BROKEN.lock().unwrap_or_else(|e| e.into_inner()).take();
    match (r, b) {
        (Err(f), _) => Err(f),
        (Ok(()), Some((call, msg))) => Err(fail(&format!("ukey__contract__{call}"), msg)),
        (Ok(()), None) => Ok(()),
    }
}

pub fn umodel<const W: u8, const F: u8>(case: &umodel::Case) -> Result<(), Fail> {
    wrap(|| umodel::run::<UK<W, F>, umodel::Val<0>, WithInlineKey>(case))
}
pub fn umodel_narrow<const W: u8, const F: u8>(case: &umodel::Case) -> Result<(), Fail> {
    wrap(|| umodel::run::<UK<W, F>, umodel::Val<3>, bbtree::Configured<WithInlineKey, bulk::Narrow>>(case))
}
pub fn umodel_sync<const W: u8, const F: u8>(case: &umodel::Case) -> Result<(), Fail> {
    wrap(|| umodel::run_sync::<UK<W, F>, umodel::Val<0>, WithInlineKey>(case))
}
pub fn annex<const W: u8, const F: u8>(case: &annex::Case) -> Result<(), Fail> {
    wrap(|| annex::run::<UK<W, F>, bbtree::Configured<WithInlineKey, bulk::Narrow>>(case))
}
pub fn synclin<const W: u8, const F: u8>(case: &synclin::Case) -> Result<(), Fail> {
    wrap(|| synclin::run::<UK<W, F>, bbtree::Configured<WithInlineKey, syncthreads::Narrow>, 4, true>(case))
}
pub fn bulk<const W: u8, const F: u8>(case: &bulk::Case) -> Result<(), Fail> {
    wrap(|| bulk::run::<UK<W, F>, bbtree::Configured<WithInlineKey, bulk::Narrow>>(case))
}
pub fn annex_threads<const W: u8, const F: u8>(case: &annex::TCase) -> Result<(), Fail> {
    wrap(|| annex::run_threads::<UK<W, F>, WithInlineKey>(case))
}
/// The sametree oracle over user keys: its failures stay `sametree__*`,
/// broken call premises are `ukey__contract__*`.
pub fn sametree<const W: u8, const F: u8>(case: &crate::sametree::Case) -> Result<(), Fail> {
    wrap_as("sametree", || crate::sametree::run::<UK<W, F>, umodel::Val<0>, WithInlineKey>(case))
}
pub fn sametree_narrow<const W: u8, const F: u8>(case: &crate::sametree::Case) -> Result<(), Fail> {
    wrap_as("sametree", || crate::sametree::run::<UK<W, F>, umodel::Val<3>, bbtree::Configured<WithInlineKey, bulk::Narrow>>(case))
}
