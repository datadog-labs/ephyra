// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! `bbtree::file`: a map written by `FileWriter` and read back by
//! `FileMap` answers every lookup, range and walk as std's `BTreeMap` of the
//! same entries does; and `FileMap::open` on damaged bytes either refuses
//! them or gives a map that may answer wrongly or panic, never anything
//! worse (run under AddressSanitizer for the "worse").

use std::collections::BTreeMap;
use std::ops::Bound;
use std::panic::{AssertUnwindSafe, catch_unwind};

use bbtree::file::{FileBuf, FileMap, FileWriter};
use bbtree::unsync::BBTreeMap;
use bbtree_properties::{Damage, FromRaw, MapSeed, RawBound, range_panics};
use ephyra::{BoxGen, Config, GenExt, TestResult, check, gens};

/// A walk over an iterator: `true` takes from the front, `false` from the
/// back.
type Walk = Vec<bool>;

#[derive(Clone, Debug, PartialEq)]
enum Probe {
    Get(u128),
    GetMany(u128, u128, u128),
    Range(RawBound, RawBound, Walk),
    Iter(Walk),
}

fn walk() -> BoxGen<Walk> {
    gens::vec(gens::bool()).len(0..=40).boxed()
}

fn probe() -> BoxGen<Probe> {
    use bbtree_properties::raw_key;
    gens::frequency(vec![
        (3, raw_key().map(Probe::Get).boxed()),
        (1, (raw_key(), raw_key(), raw_key()).map(|(a, b, c)| Probe::GetMany(a, b, c)).boxed()),
        (4, (RawBound::generator(), RawBound::generator(), walk()).map(|(s, e, w)| Probe::Range(s, e, w)).boxed()),
        (1, walk().map(Probe::Iter).boxed()),
    ])
    .boxed()
}

fn case() -> BoxGen<(MapSeed, Vec<Probe>)> {
    (MapSeed::generator(), gens::vec(probe()).len(0..=24)).boxed()
}

fn config(name: &str) -> Config {
    Config::default().name(name).tests(300)
}

/// Follows `walk` over both iterators, then drains the rest from the
/// front; the first difference, if any.
fn compare_walk<T: PartialEq + std::fmt::Debug>(
    mut ours: impl DoubleEndedIterator<Item = T>,
    mut model: impl DoubleEndedIterator<Item = T>,
    walk: &[bool],
) -> Result<(), String> {
    for (step, &front) in walk.iter().enumerate() {
        let (a, b) = if front { (ours.next(), model.next()) } else { (ours.next_back(), model.next_back()) };
        if a != b {
            return Err(format!("step {step} ({}): file map {a:?}, std {b:?}", if front { "next" } else { "next_back" }));
        }
    }
    let rest_ours: Vec<T> = ours.collect();
    let rest_model: Vec<T> = model.collect();
    if rest_ours != rest_model {
        let at = rest_ours.iter().zip(&rest_model).position(|(a, b)| a != b);
        return Err(format!(
            "after the walk: file map {} entries, std {}; first difference at {at:?}: {:?} vs {:?}",
            rest_ours.len(),
            rest_model.len(),
            at.map(|i| &rest_ours[i]),
            at.map(|i| &rest_model[i]),
        ));
    }
    Ok(())
}

macro_rules! roundtrip {
    ($name:ident, $k:ty, $v:ty) => {
        roundtrip!($name, $k, $v, bbtree::Auto);
    };
    ($name:ident, $k:ty, $v:ty, $s:ty) => {
        #[test]
        fn $name() {
            check(&config(stringify!($name)), case(), |(seed, probes)| -> TestResult {
                let model: BTreeMap<$k, $v> =
                    seed.raw_keys().map(|raw| (<$k>::from_raw(raw), <$v>::from_raw(seed.value_of(raw) as u128))).collect();
                let mut map = BBTreeMap::<$k, $v, $s>::with_strategy(<$s>::default());
                map.extend(model.iter().map(|(k, v)| (*k, *v)));
                let writer = FileWriter::from(&map);
                let from_map = writer.to_buf();
                let mut streamed = Vec::new();
                writer.write(|bytes| {
                    streamed.extend_from_slice(bytes);
                    Ok::<(), ()>(())
                }).unwrap();
                if streamed[..] != from_map[..] || writer.byte_len() != from_map.len() || writer.len() != model.len() {
                    return TestResult::Fail(format!(
                        "write streamed {} bytes, to_buf {}, byte_len {}; len {} vs {}",
                        streamed.len(), from_map.len(), writer.byte_len(), writer.len(), model.len()
                    ));
                }
                let from_ascending = FileWriter::<$k, $v, $s>::from_ascending(model.iter().map(|(k, v)| (*k, *v)))
                    .expect("std's keys ascend")
                    .to_buf();
                if from_map[..] != from_ascending[..] {
                    return TestResult::Fail(format!(
                        "FileWriter::from(&map) and from_ascending wrote different bytes ({} vs {})",
                        from_map.len(),
                        from_ascending.len()
                    ));
                }
                let file = match FileMap::<$k, $v, $s>::open(&from_map) {
                    Ok(file) => file,
                    Err(error) => return TestResult::Fail(format!("open refused a fresh file map: {error}")),
                };
                if file.len() != model.len() {
                    return TestResult::Fail(format!("len {} vs {}", file.len(), model.len()));
                }
                if file.first_key_value() != model.first_key_value() || file.last_key_value() != model.last_key_value() {
                    return TestResult::Fail(format!(
                        "first/last {:?} {:?} vs {:?} {:?}",
                        file.first_key_value(),
                        file.last_key_value(),
                        model.first_key_value(),
                        model.last_key_value()
                    ));
                }
                if file.iter().len() != model.len() {
                    return TestResult::Fail(format!("iter().len() {} vs {}", file.iter().len(), model.len()));
                }
                // Every stored key, and its neighbours.
                for (k, v) in model.iter().step_by((model.len() / 64).max(1)) {
                    if file.get(k) != Some(v) || !file.contains_key(k) || file.get_key_value(k) != Some((k, v)) {
                        return TestResult::Fail(format!("stored key {k:?}: get {:?}, want {v:?}", file.get(k)));
                    }
                }
                for probe in &probes {
                    let failure = match probe {
                        Probe::Get(raw) => {
                            let k = <$k>::from_raw(*raw);
                            (file.get(&k) != model.get(&k)).then(|| format!("get({k:?}) {:?} vs {:?}", file.get(&k), model.get(&k)))
                        }
                        Probe::GetMany(a, b, c) => {
                            let keys = [<$k>::from_raw(*a), <$k>::from_raw(*b), <$k>::from_raw(*c)];
                            let ours = file.get_many([&keys[0], &keys[1], &keys[2]]);
                            let want = [model.get(&keys[0]), model.get(&keys[1]), model.get(&keys[2])];
                            (ours != want).then(|| format!("get_many({keys:?}) {ours:?} vs {want:?}"))
                        }
                        Probe::Range(start, end, walk) => {
                            let (s, e): (Bound<$k>, Bound<$k>) = (start.to(), end.to());
                            // std checks the bounds only when the map has a root.
                            if range_panics(&s, &e) {
                                let ours = catch_unwind(AssertUnwindSafe(|| file.range((s, e)).count())).is_err();
                                let std = catch_unwind(AssertUnwindSafe(|| model.range((s, e)).count())).is_err();
                                (ours != std).then(|| format!("range({s:?}, {e:?}): file map panics {ours}, std {std}"))
                            } else {
                                compare_walk(file.range((s, e)), model.range((s, e)), walk)
                                    .err()
                                    .map(|why| format!("range({s:?}, {e:?}): {why}"))
                            }
                        }
                        Probe::Iter(walk) => compare_walk(file.iter(), model.iter(), walk).err().map(|why| format!("iter: {why}")),
                    };
                    if let Some(failure) = failure {
                        return TestResult::Fail(failure);
                    }
                }
                TestResult::Pass
            })
            .tap_print()
            .assert_passed();
        }
    };
}

// Inline-key types of `Auto` (u64, i64, u128, i128, the byte arrays it
// names), and keys that compare whole.
roundtrip!(roundtrip_u64_u64, u64, u64);
roundtrip!(roundtrip_i64_u8, i64, u8);
roundtrip!(roundtrip_u128_u32, u128, u32);
roundtrip!(roundtrip_i128_i128, i128, i128);
roundtrip!(roundtrip_arr16_u64, [u8; 16], u64);
roundtrip!(roundtrip_arr20_arr3, [u8; 20], [u8; 3]);
roundtrip!(roundtrip_arr32_u16, [u8; 32], u16);
roundtrip!(roundtrip_u32_u64, u32, u64);
roundtrip!(roundtrip_u8_arr64, u8, [u8; 64]);
roundtrip!(roundtrip_i16_unit_like, i16, [u8; 0]);
roundtrip!(roundtrip_arr7_arr200, [u8; 7], [u8; 200]);

/// Node configs other than the default: the file format writes their page
/// sizes, and a reader must find its pages by them.
struct BigLeaves;
impl bbtree::NodeConfig for BigLeaves {
    const LEAF_PAGE_SIZE: usize = 8192;
}
struct TinyInternals;
impl bbtree::NodeConfig for TinyInternals {
    const INTERNAL_PAGE_SIZE: usize = 64;
    const PLAIN_INTERNAL_PAGE_SIZE: usize = 64;
}
struct BigInternals;
impl bbtree::NodeConfig for BigInternals {
    const INTERNAL_PAGE_SIZE: usize = 4096;
    const PLAIN_INTERNAL_PAGE_SIZE: usize = 8192;
}
type Cfg<C> = bbtree::Configured<bbtree::Auto, C>;
type CfgInline<C> = bbtree::Configured<bbtree::WithInlineKey, C>;

roundtrip!(roundtrip_cfg_big_leaves_u64, u64, u64, Cfg<BigLeaves>);
roundtrip!(roundtrip_cfg_tiny_internals_u64, u64, u32, Cfg<TinyInternals>);
roundtrip!(roundtrip_cfg_tiny_internals_u16, u16, u64, Cfg<TinyInternals>);
roundtrip!(roundtrip_cfg_big_internals_arr20, [u8; 20], u8, Cfg<BigInternals>);
roundtrip!(roundtrip_cfg_big_internals_u32, u32, [u8; 40], Cfg<BigInternals>);
roundtrip!(roundtrip_with_inline_key_u64, u64, u64, bbtree::WithInlineKey);
roundtrip!(roundtrip_with_inline_key_arr32, [u8; 32], u64, CfgInline<TinyInternals>);

/// Reads everything a damaged map offers, bounded, under `catch_unwind`:
/// panics are allowed, so only UB (ASan) or a hang (the timeout) fails.
fn exercise<K: FromRaw, V: std::fmt::Debug>(file: &FileMap<'_, K, V>, probes: &[u128]) -> usize
where
    K: bbtree::file::Storable + bbtree::file::PlainBytes,
    V: bbtree::file::Storable,
{
    let mut seen = 0usize;
    let bound = file.len().min(1 << 20) + 8;
    let _ = catch_unwind(AssertUnwindSafe(|| {
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
    }));
    seen
}

macro_rules! damaged {
    ($name:ident, $k:ty, $v:ty) => {
        #[test]
        fn $name() {
            let generator = (MapSeed::generator(), gens::vec(Damage::generator()).len(1..=4), gens::vec(bbtree_properties::raw_key()).len(0..=8));
            let watchdog = bbtree_properties::Watchdog::new(stringify!($name), std::time::Duration::from_secs(10));
            check(&config(stringify!($name)), generator, |input| -> TestResult { watchdog.run(&input, || {
                let (seed, damage, probes) = &input;
                let map: BBTreeMap<$k, $v> =
                    seed.raw_keys().map(|raw| (<$k>::from_raw(raw), <$v>::from_raw(seed.value_of(raw) as u128))).collect();
                let fresh = FileWriter::from(&map).to_buf();
                let mut buf = FileBuf::zeroed(fresh.len(), FileMap::<$k, $v>::alignment());
                buf.copy_from_slice(&fresh);
                let mut len = buf.len();
                for d in damage {
                    len = d.apply(&mut buf, len);
                }
                if let Ok(file) = FileMap::<$k, $v>::open(&buf[..len]) {
                    ephyra::classify(true, "opened");
                    let changed = catch_unwind(AssertUnwindSafe(|| {
                        file.len() != map.len() || file.iter().take(map.len() + 1).ne(map.iter())
                    }))
                    .unwrap_or(true);
                    ephyra::classify(changed, "opened, answers changed");
                    exercise(&file, probes);
                }
                TestResult::Pass
            })})
            .tap_print()
            .assert_passed();
        }
    };
}

damaged!(damaged_u64_u64, u64, u64);
damaged!(damaged_arr20_u32, [u8; 20], u32);
damaged!(damaged_u16_u8, u16, u8);
damaged!(damaged_i128_arr24, i128, [u8; 24]);

/// Prints the report of a passing run too, for its labels and counts.
trait TapPrint {
    fn tap_print(self) -> Self;
}

impl TapPrint for ephyra::Report {
    fn tap_print(self) -> Self {
        eprintln!("{self}");
        self
    }
}
