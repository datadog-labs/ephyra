// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! `bbtree::file`: a map written by `FileWriter` and read back by
//! `FileMap` answers every lookup, range and walk as std's `BTreeMap` of the
//! same entries does; and `FileMap::open` on damaged bytes either refuses
//! them or gives a map that may answer wrongly or panic, never anything
//! worse (the ASan build checks the "worse", the supervisor the hangs).

use std::collections::BTreeMap;
use std::ops::Bound;
use std::panic::{AssertUnwindSafe, catch_unwind};

use bbtree::file::{FileBuf, FileMap, FileWriter, PlainBytes, Storable};
use bbtree::unsync::BBTreeMap;
use bbtree::{InlineKeyStrategy, NodeConfig};
use ephyra::{BoxGen, GenExt, classify, gens};
use serde::{Deserialize, Serialize};

use crate::common::{Damage, Fail, FromRaw, MapSeed, RawBound, compare_walk, ensure, fail, quietly, range_panics, raw_key, selftest};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Probe {
    Get(u128),
    GetMany(u128, u128, u128),
    /// Bounds and a walk: `true` takes from the front, `false` from the back.
    Range(RawBound, RawBound, Vec<bool>),
    Iter(Vec<bool>),
}

pub type RoundtripCase = (MapSeed, Vec<Probe>);
pub type DamagedCase = (MapSeed, Vec<Damage>, Vec<u128>);

fn walk() -> BoxGen<Vec<bool>> {
    gens::vec(gens::bool()).len(0..=40).boxed()
}

fn probe() -> BoxGen<Probe> {
    gens::frequency(vec![
        (3, raw_key().map(Probe::Get).boxed()),
        (1, (raw_key(), raw_key(), raw_key()).map(|(a, b, c)| Probe::GetMany(a, b, c)).boxed()),
        (4, (RawBound::generator(), RawBound::generator(), walk()).map(|(s, e, w)| Probe::Range(s, e, w)).boxed()),
        (1, walk().map(Probe::Iter).boxed()),
    ])
    .boxed()
}

pub fn roundtrip_gen() -> BoxGen<RoundtripCase> {
    (MapSeed::generator(), gens::vec(probe()).len(0..=24)).boxed()
}

pub fn damaged_gen() -> BoxGen<DamagedCase> {
    (MapSeed::generator(), gens::vec(Damage::generator()).len(1..=4), gens::vec(raw_key()).len(0..=8)).boxed()
}

pub fn roundtrip<K, V, S>(case: &RoundtripCase) -> Result<(), Fail>
where
    K: FromRaw + PlainBytes + Storable<InWriter = K>,
    V: FromRaw + PlainBytes + Storable<InWriter = V> + PartialEq,
    S: InlineKeyStrategy<K> + Default,
{
    let (seed, probes) = case;
    let model: BTreeMap<K, V> =
        seed.raw_keys().map(|raw| (K::from_raw(raw), V::from_raw(seed.value_of(raw) as u128))).collect();
    let mut map = BBTreeMap::<K, V, S>::with_strategy(S::default());
    map.extend(model.iter().map(|(k, v)| (*k, *v)));
    let writer = FileWriter::from(&map);
    let from_map = writer.to_buf();
    let mut streamed = Vec::new();
    writer
        .write(|bytes| {
            streamed.extend_from_slice(bytes);
            Ok::<(), ()>(())
        })
        .unwrap();
    ensure(
        streamed[..] == from_map[..] && writer.byte_len() == from_map.len() && writer.len() == model.len(),
        "filemap__mismatch__write_stream",
        || {
            format!(
                "write streamed {} bytes, to_buf {}, byte_len {}; len {} vs {}",
                streamed.len(),
                from_map.len(),
                writer.byte_len(),
                writer.len(),
                model.len()
            )
        },
    )?;
    let from_ascending = FileWriter::<K, V, S>::from_ascending(model.iter().map(|(k, v)| (*k, *v)))
        .map_err(|e| fail("filemap__refused__from_ascending", format!("std's keys refused as not ascending: {e:?}")))?
        .to_buf();
    ensure(from_map[..] == from_ascending[..], "filemap__mismatch__from_ascending", || {
        format!("FileWriter::from(&map) and from_ascending wrote different bytes ({} vs {})", from_map.len(), from_ascending.len())
    })?;
    if selftest("break") && model.len() >= 100 {
        return Err(fail("filemap__mismatch__get", format!("BBZ_SELFTEST_BREAK: a map of {} entries", model.len())));
    }
    let file = FileMap::<K, V, S>::open(&from_map)
        .map_err(|e| fail("filemap__open_refused__fresh", format!("open refused a fresh file map: {e}")))?;
    ensure(file.len() == model.len(), "filemap__mismatch__len", || format!("len {} vs {}", file.len(), model.len()))?;
    ensure(
        file.first_key_value() == model.first_key_value() && file.last_key_value() == model.last_key_value(),
        "filemap__mismatch__first_last",
        || {
            format!(
                "first/last {:?} {:?} vs {:?} {:?}",
                file.first_key_value(),
                file.last_key_value(),
                model.first_key_value(),
                model.last_key_value()
            )
        },
    )?;
    ensure(file.iter().len() == model.len(), "filemap__mismatch__iter_len", || {
        format!("iter().len() {} vs {}", file.iter().len(), model.len())
    })?;
    for (k, v) in model.iter().step_by((model.len() / 64).max(1)) {
        ensure(
            file.get(k) == Some(v) && file.contains_key(k) && file.get_key_value(k) == Some((k, v)),
            "filemap__mismatch__get_stored",
            || format!("stored key {k:?}: get {:?}, want {v:?}", file.get(k)),
        )?;
    }
    for probe in probes {
        match probe {
            Probe::Get(raw) => {
                let k = K::from_raw(*raw);
                ensure(file.get(&k) == model.get(&k), "filemap__mismatch__get", || {
                    format!("get({k:?}) {:?} vs {:?}", file.get(&k), model.get(&k))
                })?;
            }
            Probe::GetMany(a, b, c) => {
                let keys = [K::from_raw(*a), K::from_raw(*b), K::from_raw(*c)];
                let ours = file.get_many([&keys[0], &keys[1], &keys[2]]);
                let want = [model.get(&keys[0]), model.get(&keys[1]), model.get(&keys[2])];
                ensure(ours == want, "filemap__mismatch__get_many", || format!("get_many({keys:?}) {ours:?} vs {want:?}"))?;
            }
            Probe::Range(start, end, walk) => {
                let (s, e): (Bound<K>, Bound<K>) = (start.to(), end.to());
                if range_panics(&s, &e) {
                    let ours = catch_unwind(AssertUnwindSafe(|| file.range((s, e)).count())).is_err();
                    let std = catch_unwind(AssertUnwindSafe(|| model.range((s, e)).count())).is_err();
                    ensure(ours == std, "filemap__panic_mismatch__range", || {
                        format!("range({s:?}, {e:?}): file map panics {ours}, std {std}")
                    })?;
                } else {
                    compare_walk(file.range((s, e)), model.range((s, e)), walk)
                        .map_err(|why| fail("filemap__mismatch__range", format!("range({s:?}, {e:?}): {why}")))?;
                }
            }
            Probe::Iter(walk) => {
                compare_walk(file.iter(), model.iter(), walk).map_err(|why| fail("filemap__mismatch__iter", format!("iter: {why}")))?;
            }
        }
    }
    Ok(())
}

pub fn damaged<K, V>(case: &DamagedCase) -> Result<(), Fail>
where
    K: FromRaw + PlainBytes + Storable<InWriter = K>,
    V: FromRaw + PlainBytes + Storable<InWriter = V> + PartialEq,
    bbtree::Auto: InlineKeyStrategy<K>,
{
    let (seed, damage, probes) = case;
    let map: BBTreeMap<K, V> =
        seed.raw_keys().map(|raw| (K::from_raw(raw), V::from_raw(seed.value_of(raw) as u128))).collect();
    let fresh = FileWriter::from(&map).to_buf();
    let mut buf = FileBuf::zeroed(fresh.len(), FileMap::<K, V>::alignment());
    buf.copy_from_slice(&fresh);
    let mut len = buf.len();
    for d in damage {
        len = d.apply(&mut buf, len);
    }
    // The checker itself must not panic: it refuses or accepts.
    let opened = catch_unwind(AssertUnwindSafe(|| FileMap::<K, V>::open(&buf[..len])))
        .map_err(|_| fail("filemapdmg__panic__open", "FileMap::open panicked on damaged bytes"))?;
    if let Ok(file) = opened {
        classify(true, "opened");
        let changed = quietly(|| file.len() != map.len() || file.iter().take(map.len() + 1).ne(map.iter())).unwrap_or(true);
        classify(changed, "answers_changed");
        // Reads everything the damaged map offers, bounded; panics are allowed.
        let bound = file.len().min(1 << 20) + 8;
        let _ = quietly(|| {
            let mut seen = 0usize;
            let _ = file.first_key_value();
            let _ = file.last_key_value();
            for raw in probes {
                let k = K::from_raw(*raw);
                seen += file.get(&k).is_some() as usize;
                seen += file.range(k..).take(4).count();
                seen += file.range(..k).rev().take(4).count();
            }
            seen += file.iter().take(bound).count();
            seen += file.iter().rev().take(bound).count();
            let _ = format!("{:?}", file.iter().take(16).collect::<Vec<_>>());
            seen
        });
    }
    Ok(())
}

/// Node configs other than the default: the file format writes their page
/// sizes, and a reader must find its pages by them.
pub struct BigLeaves;
impl NodeConfig for BigLeaves {
    const LEAF_PAGE_SIZE: usize = 8192;
}
pub struct TinyInternals;
impl NodeConfig for TinyInternals {
    const INTERNAL_PAGE_SIZE: usize = 64;
    const PLAIN_INTERNAL_PAGE_SIZE: usize = 64;
}
pub struct BigInternals;
impl NodeConfig for BigInternals {
    const INTERNAL_PAGE_SIZE: usize = 4096;
    const PLAIN_INTERNAL_PAGE_SIZE: usize = 8192;
}
pub type Cfg<C> = bbtree::Configured<bbtree::Auto, C>;
pub type CfgInline<C> = bbtree::Configured<bbtree::WithInlineKey, C>;
