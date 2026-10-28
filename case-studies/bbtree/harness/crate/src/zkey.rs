// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! zkey: the exact oracles (umodel, sametree, shlin, synclin, syncthreads)
//! over byte-string keys of another shape. Every other property's byte
//! strings are a length byte and big-endian digits (umodel's `bytes_of`)
//! or formatted numbers (syncthreads' `Key for String`): no key is a prefix
//! of another, none ends in zero bytes, none is empty, and no key equals or
//! is a prefix of the tree-wide shared prefix (D50).
//!
//! Here keys are strings over {0x00, 0x01, 'a', 0x7f} of up to `DEPTH` bytes, in
//! trie preorder: rank 0 is the empty key, then "\0", "\0\0", ... So
//! neighbours are prefixes of each other, and keys differ only in trailing
//! zero bytes, which a zero-padded inline word (D18) or a window past a
//! shared prefix cannot tell apart: only lengths and `cmp_from` can. The
//! order is the byte order, so it is monotone in the rank, as the threaded
//! properties need for ranges by number. Under shrink the lowest numbers
//! are the prefixes of the shared prefix itself ("t" .. "tenant/7"), then
//! the shared prefix as a whole key ("tenant/7/" + the empty rank).
//!
//! A process-wide switch (`ON`) turns the shape on for one case: each
//! worker runs one case at a time, and the threads a case spawns see it.
//! The oracles and their checks are the same; signatures are renamed
//! `z<oracle>__<kind>__<area>`.

use std::sync::atomic::{AtomicBool, Ordering};

use bbtree::{BBTreeMap, InlineKeyStrategy};

use crate::common::Fail;

static ON: AtomicBool = AtomicBool::new(false);

pub fn on() -> bool {
    ON.load(Ordering::Relaxed)
}

const ALPHABET: [u8; 4] = [0x00, 0x01, b'a', 0x7f];
const DEPTH: usize = 11;
pub const SHARED: &[u8] = b"tenant/7/";

/// Keys in a subtree of the trie whose root is at depth `d` (itself
/// included).
const fn subtree(d: usize) -> u64 {
    let mut total = 0;
    let mut p = 1;
    let mut i = 0;
    while i <= DEPTH - d {
        total += p;
        p *= ALPHABET.len() as u64;
        i += 1;
    }
    total
}

/// Number of keys: 5592405 (synclin's hot keys reach 1.5M).
pub const COUNT: u64 = subtree(0);

/// The key of preorder rank `n` (taken modulo [`COUNT`]).
pub fn unrank(n: u64) -> Vec<u8> {
    let mut n = n % COUNT;
    let mut k = Vec::new();
    while n > 0 {
        n -= 1;
        let below = subtree(k.len() + 1);
        k.push(ALPHABET[(n / below) as usize]);
        n %= below;
    }
    k
}

/// The rank of a key `unrank` makes; `None` for any other.
pub fn rank(k: &[u8]) -> Option<u64> {
    if k.len() > DEPTH {
        return None;
    }
    let mut n = 0;
    for (d, b) in k.iter().enumerate() {
        let c = ALPHABET.iter().position(|a| a == b)? as u64;
        n += 1 + c * subtree(d + 1);
    }
    Some(n)
}

/// umodel's keys (`bytes_of`) in this shape, by mode: bare; behind the
/// shared prefix; behind 300 bytes (saturated hash tags); a mix that also
/// puts zero runs at the inline-word edges (7, 8 bytes), 255-byte heads,
/// and keys that are prefixes of the others' prefix.
pub fn bytes_of(raw: u128, mode: u8) -> Vec<u8> {
    let n = (raw as u64) ^ ((raw >> 64) as u64);
    let mut k = match mode % 4 {
        0 => Vec::new(),
        1 => SHARED.to_vec(),
        2 => vec![b'p'; 300],
        _ => match raw % 6 {
            0 => Vec::new(),
            1 => SHARED.to_vec(),
            2 => vec![0; 7],
            3 => vec![0; 8],
            4 => vec![b'q'; 255],
            _ => return SHARED[..(n % (SHARED.len() as u64 + 1)) as usize].to_vec(),
        },
    };
    k.extend(unrank(n));
    k
}

/// syncthreads' key numbers in this shape (see the module docs).
pub fn of_number(n: u32, shrink: bool) -> Vec<u8> {
    if !shrink {
        return unrank(u64::from(n));
    }
    let outside = SHARED.len() as u32 - 1;
    if n < outside {
        return SHARED[..n as usize + 1].to_vec();
    }
    let mut k = SHARED.to_vec();
    k.extend(unrank(u64::from(n - outside)));
    k
}

pub fn number_of(k: &[u8], shrink: bool) -> u32 {
    if !shrink {
        return rank(k).map_or(u32::MAX, |n| n as u32);
    }
    let outside = SHARED.len() as u32 - 1;
    match k.strip_prefix(SHARED) {
        Some(rest) => rank(rest).map_or(u32::MAX, |n| n as u32 + outside),
        None if !k.is_empty() && SHARED.starts_with(k) => k.len() as u32 - 1,
        None => u32::MAX,
    }
}

/// Keys never preloaded under shrink: the prefixes of the shared prefix and
/// the shared prefix itself.
pub fn outside(n: u32) -> bool {
    n <= SHARED.len() as u32 - 1
}

fn retag(mut f: Fail) -> Fail {
    f.sig = format!("z{}", f.sig);
    f
}

/// Turns the shape on for `f`, off again after (also on unwind).
fn with<T>(f: impl FnOnce() -> T) -> T {
    struct Off;
    impl Drop for Off {
        fn drop(&mut self) {
            ON.store(false, Ordering::Relaxed);
        }
    }
    ON.store(true, Ordering::Relaxed);
    let _off = Off;
    f()
}

pub fn umodel<K: crate::umodel::UKey, V: crate::umodel::UVal, S: InlineKeyStrategy<K> + Default>(case: &crate::umodel::Case) -> Result<(), Fail> {
    with(|| crate::umodel::run::<K, V, S>(case)).map_err(retag)
}

pub fn umodel_sync<K: crate::umodel::UKey + Send + Sync, V: crate::umodel::UVal, S: InlineKeyStrategy<K> + Default>(case: &crate::umodel::Case) -> Result<(), Fail> {
    with(|| crate::umodel::run_sync::<K, V, S>(case)).map_err(retag)
}

pub fn sametree<K: crate::umodel::UKey, V: crate::umodel::UVal, S: InlineKeyStrategy<K> + Default>(case: &crate::sametree::Case) -> Result<(), Fail> {
    with(|| crate::sametree::run::<K, V, S>(case)).map_err(retag)
}

pub fn shlin<K: crate::syncthreads::Key, S: InlineKeyStrategy<K> + Default + Send + Sync, const N: usize, const SHRINK: bool>(case: &crate::shlin::Case) -> Result<(), Fail>
where
    BBTreeMap<K, crate::shlin::V<N>, S>: Sync,
{
    with(|| crate::shlin::run::<K, S, N, SHRINK>(case)).map_err(retag)
}

pub fn synclin<K: crate::syncthreads::Key, S: InlineKeyStrategy<K> + Default + Send + Sync, const N: usize, const SHRINK: bool>(case: &crate::synclin::Case) -> Result<(), Fail>
where
    BBTreeMap<K, crate::synclin::W<N>, S>: Sync,
{
    with(|| crate::synclin::run::<K, S, N, SHRINK>(case)).map_err(retag)
}

pub fn syncthreads<K: crate::syncthreads::Key, S: InlineKeyStrategy<K> + Default + Send + Sync, const SHRINK: bool>(case: &crate::syncthreads::Case) -> Result<(), Fail>
where
    BBTreeMap<K, u64, S>: Sync,
{
    with(|| crate::syncthreads::run::<K, S, SHRINK>(case)).map_err(retag)
}

pub fn shlinr<K: crate::syncthreads::Key, S: InlineKeyStrategy<K> + Send + Sync, const N: usize, const SHRINK: bool>(case: &crate::shlin::RCase) -> Result<(), Fail>
where
    BBTreeMap<K, crate::shlin::V<N>, S, crate::synclin::Flaky>: Sync,
{
    with(|| crate::shlin::run_refuse::<K, S, N, SHRINK>(case)).map_err(retag)
}

pub fn synclinr<K: crate::syncthreads::Key, S: InlineKeyStrategy<K> + Send + Sync, const N: usize, const SHRINK: bool>(case: &crate::synclin::RCase) -> Result<(), Fail>
where
    BBTreeMap<K, crate::synclin::W<N>, S, crate::synclin::Flaky>: Sync,
{
    with(|| crate::synclin::run_refuse::<K, S, N, SHRINK>(case)).map_err(retag)
}

pub fn bulk<K: crate::bulk::Key, S: InlineKeyStrategy<K> + Default>(case: &crate::bulk::Case) -> Result<(), Fail> {
    with(|| crate::bulk::run::<K, S>(case)).map_err(retag)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preorder() {
        let keys: Vec<Vec<u8>> = (0..COUNT).map(unrank).collect();
        assert!(keys.windows(2).all(|w| w[0] < w[1]));
        assert!(keys.iter().enumerate().all(|(i, k)| rank(k) == Some(i as u64)));
        for shrink in [false, true] {
            let ks: Vec<Vec<u8>> = (0..130_000).map(|n| of_number(n, shrink)).collect();
            assert!(ks.windows(2).all(|w| w[0] < w[1]));
            assert!(ks.iter().enumerate().all(|(i, k)| number_of(k, shrink) == i as u32));
        }
    }
}
