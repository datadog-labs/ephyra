// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! `bbtree-mmap`: a `BBTreeMapFile` written from a map and opened every way
//! (read whole, mapped, embedded at an offset in a larger file) answers as
//! std's `BTreeMap` does; and `BBTreeMapFile::read`, which checks the file,
//! on damaged bytes refuses them or gives a map that may answer wrongly or
//! panic, never anything worse.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::ops::Bound;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use bbtree_mmap::BBTreeMapFile;
use ephyra::{BoxGen, GenExt, classify, gens};
use serde::{Deserialize, Serialize};

use crate::common::{Damage, Fail, ensure, fail, quietly, range_panics};

static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

/// A file in the temp directory, removed when dropped. Its name is random and
/// created exclusively: the controller's sandbox gives every slot its own PID
/// namespace and one shared /tmp, so a name from the process id collides
/// across slots (one worker truncating another's mapped file: SIGBUS, torn
/// reads).
pub struct TempPath(pub PathBuf);

impl TempPath {
    pub fn new() -> Self {
        use std::hash::{BuildHasher, Hasher};
        loop {
            let n = NEXT_FILE.fetch_add(1, Ordering::Relaxed);
            let mut h = std::collections::hash_map::RandomState::new().build_hasher();
            h.write_u64(n);
            h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
            let path = std::env::temp_dir().join(format!("bbz-{:016x}-{n}", h.finish()));
            if fs::OpenOptions::new().write(true).create_new(true).open(&path).is_ok() {
                return TempPath(path);
            }
        }
    }
}

impl Drop for TempPath {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Byte strings from a small alphabet, sharing prefixes, a few long.
pub fn bytes(long: usize) -> BoxGen<Vec<u8>> {
    let alphabet = gens::pick_from(vec![0u8, 1, b'a', b'b', 0x7f, 0xff]);
    gens::frequency(vec![
        (6, gens::vec(alphabet.clone()).len(0..=10).boxed()),
        (2, gens::vec(alphabet.clone()).len(0..=40).boxed()),
        (
            1,
            (gens::vec(alphabet).len(0..=3), gens::int(0usize..=long), gens::int(0u8..=255))
                .iso(
                    |(mut head, len, fill): (Vec<u8>, usize, u8)| {
                        head.resize(len.max(head.len()), fill);
                        head
                    },
                    |k: &Vec<u8>| (k[..k.len().min(3)].to_vec(), k.len(), k.last().copied().unwrap_or(0)),
                )
                .boxed(),
        ),
    ])
    .boxed()
}

/// Text: the byte strings' alphabet as chars, some multi-byte.
pub fn text(long: usize) -> BoxGen<String> {
    let chars = gens::pick_from(vec!['\0', 'a', 'b', 'é', '\u{7f}', '\u{10FFFF}', '中']);
    gens::frequency(vec![(6, gens::string_len(chars.clone(), 0..=10).boxed()), (1, gens::string_len(chars, 0..=long).boxed())])
        .boxed()
}

pub fn ints(hi: u64) -> BoxGen<u64> {
    gens::int(0u64..=hi).boxed()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Probe<K> {
    Get(K),
    Range(Bound<K>, Bound<K>, Vec<bool>),
}

pub fn probes<K: Clone + std::fmt::Debug + PartialEq + Send + Sync + 'static>(key: BoxGen<K>) -> BoxGen<Vec<Probe<K>>> {
    let bound = (gens::int(0u8..=4), key.clone())
        .map(|(kind, k)| match kind {
            0 | 1 => Bound::Included(k),
            2 | 3 => Bound::Excluded(k),
            _ => Bound::Unbounded,
        })
        .boxed();
    gens::vec(
        gens::frequency(vec![
            (2, key.map(Probe::Get).boxed()),
            (2, (bound.clone(), bound, gens::vec(gens::bool()).len(0..=30)).map(|(s, e, w)| Probe::Range(s, e, w)).boxed()),
        ])
        .boxed(),
    )
    .len(0..=16)
    .boxed()
}

/// A bound on keys as a bound on the query type.
pub fn qb<K: BorrowQ<Q>, Q: ?Sized>(b: &Bound<K>) -> Bound<&Q> {
    match b {
        Bound::Included(k) => Bound::Included(k.borrow_q()),
        Bound::Excluded(k) => Bound::Excluded(k.borrow_q()),
        Bound::Unbounded => Bound::Unbounded,
    }
}

/// A key as the query type a lookup takes.
pub trait BorrowQ<Q: ?Sized> {
    fn borrow_q(&self) -> &Q;
}
impl BorrowQ<[u8]> for Vec<u8> {
    fn borrow_q(&self) -> &[u8] {
        self
    }
}
impl BorrowQ<str> for String {
    fn borrow_q(&self) -> &str {
        self
    }
}
impl BorrowQ<u64> for u64 {
    fn borrow_q(&self) -> &u64 {
        self
    }
}

/// What a lookup hands back for an owned key or value.
pub trait View {
    type Out<'a>: PartialEq + std::fmt::Debug
    where
        Self: 'a;
    fn view(&self) -> Self::Out<'_>;
}
impl View for Vec<u8> {
    type Out<'a> = &'a [u8];
    fn view(&self) -> &[u8] {
        self
    }
}
impl View for String {
    type Out<'a> = &'a str;
    fn view(&self) -> &str {
        self
    }
}
impl View for u64 {
    type Out<'a> = &'a u64;
    fn view(&self) -> &u64 {
        self
    }
}
fn view<T: View>(t: &T) -> T::Out<'_> {
    t.view()
}

/// Entries, probes, and the offset the embedded copy is written at.
pub type RoundtripCase<K, V> = (Vec<(K, V)>, Vec<Probe<K>>, u64);
/// Entries, damage, probe keys.
pub type DamagedCase<K, V> = (Vec<(K, V)>, Vec<Damage>, Vec<K>);

/// Opens the file of the entries every way and checks each against std.
macro_rules! roundtrip {
    ($fname:ident, $gname:ident, $k:ty, $v:ty, $q:ty, $keys:expr, $values:expr) => {
        pub fn $gname() -> BoxGen<RoundtripCase<$k, $v>> {
            (gens::vec(($keys, $values)).len(0..=2_000), probes($keys), gens::int(0u64..=5000)).boxed()
        }

        pub fn $fname(case: &RoundtripCase<$k, $v>) -> Result<(), Fail> {
            let (entries, probes, offset) = case;
            let offset = *offset;
            let model: BTreeMap<$k, $v> = entries.iter().cloned().collect();
            let whole = TempPath::new();
            BBTreeMapFile::<$k, $v>::create(&whole.0, &model).map_err(|e| fail("mmap__error__create", format!("create: {e}")))?;
            // The same map embedded after `offset` bytes of other data, with
            // more data after it that write_at cuts.
            let embedded = TempPath::new();
            let range = {
                let mut file =
                    OpenOptions::new().read(true).write(true).create(true).truncate(true).open(&embedded.0).unwrap();
                file.write_all(&vec![0xEE; offset as usize + 100]).unwrap();
                BBTreeMapFile::<$k, $v>::write_at(&mut file, offset, &model)
                    .map_err(|e| fail("mmap__error__write_at", format!("write_at: {e}")))?
            };
            ensure(range.start >= offset, "mmap__offset__write_at", || format!("write_at returned {range:?} before the offset {offset}"))?;
            let prefix_kept = fs::read(&embedded.0).unwrap()[..offset as usize].iter().all(|&b| b == 0xEE);
            ensure(prefix_kept, "mmap__prefix__write_at", || "write_at changed bytes before its offset".into())?;
            let opened: Vec<(&str, std::io::Result<BBTreeMapFile<$k, $v>>)> = vec![
                ("read", BBTreeMapFile::<$k, $v>::read(&whole.0)),
                ("mmap", unsafe { BBTreeMapFile::<$k, $v>::mmap(&whole.0).open() }),
                ("read_range", BBTreeMapFile::<$k, $v>::read_range(&mut File::open(&embedded.0).unwrap(), range.clone())),
                ("mmap_range", unsafe { BBTreeMapFile::<$k, $v>::mmap(&embedded.0).range(range.clone()).open() }),
            ];
            for (how, file) in opened {
                let file = file.map_err(|e| fail(&format!("mmap__open_failed__{how}"), format!("{how}: open failed: {e}")))?;
                ensure(file.len() == model.len(), "mmap__mismatch__len", || format!("{how}: len {} vs {}", file.len(), model.len()))?;
                let want: Vec<_> = model.iter().map(|(k, v)| (view(k), view(v))).collect();
                let got: Vec<_> = file.iter().collect();
                ensure(got == want, "mmap__mismatch__iter", || format!("{how}: iter differs ({} vs {} entries)", got.len(), want.len()))?;
                let got_rev: Vec<_> = file.iter().rev().collect();
                ensure(got_rev.iter().rev().eq(want.iter()), "mmap__mismatch__iter_rev", || format!("{how}: iter().rev() differs"))?;
                for (k, v) in model.iter().step_by((model.len() / 32).max(1)) {
                    let q: &$q = k.borrow_q();
                    ensure(file.get(q) == Some(view(v)), "mmap__mismatch__get_stored", || format!("{how}: get({k:?}) {:?}", file.get(q)))?;
                }
                for probe in probes {
                    match probe {
                        Probe::Get(k) => {
                            let q: &$q = k.borrow_q();
                            let (got, want) = (file.get(q), model.get(k).map(view));
                            ensure(got == want, "mmap__mismatch__get", || format!("{how}: get({k:?}) {got:?} vs {want:?}"))?;
                            let (got, want) = (file.get_key_value(q), model.get_key_value(k).map(|(k, v)| (view(k), view(v))));
                            ensure(got == want, "mmap__mismatch__get_key_value", || {
                                format!("{how}: get_key_value({k:?}) {got:?} vs {want:?}")
                            })?;
                        }
                        Probe::Range(s, e, walk) => {
                            if range_panics(s, e) {
                                // std checks the bounds only when it has a root.
                                let ours = catch_unwind(AssertUnwindSafe(|| {
                                    file.range::<$q, _>((qb::<$k, $q>(s), qb::<$k, $q>(e))).count()
                                }))
                                .is_err();
                                ensure(ours || model.is_empty(), "mmap__panic_mismatch__range", || {
                                    format!("{how}: range({s:?}, {e:?}) did not panic, std's does")
                                })?;
                                continue;
                            }
                            let mut ours = file.range::<$q, _>((qb::<$k, $q>(s), qb::<$k, $q>(e)));
                            let mut want = model.range((s.clone(), e.clone())).map(|(k, v)| (view(k), view(v)));
                            for (i, &front) in walk.iter().enumerate() {
                                let (a, b) = if front { (ours.next(), want.next()) } else { (ours.next_back(), want.next_back()) };
                                ensure(a == b, "mmap__mismatch__range", || format!("{how}: range({s:?}, {e:?}) step {i}: {a:?} vs {b:?}"))?;
                            }
                            ensure(ours.eq(want), "mmap__mismatch__range", || format!("{how}: range({s:?}, {e:?}) differs after the walk"))?;
                        }
                    }
                }
            }
            Ok(())
        }
    };
}

roundtrip!(roundtrip_bytes_bytes, roundtrip_bytes_bytes_gen, Vec<u8>, Vec<u8>, [u8], bytes(3000), bytes(9000));
roundtrip!(roundtrip_text_text, roundtrip_text_text_gen, String, String, str, text(1500), text(3000));
roundtrip!(roundtrip_u64_bytes, roundtrip_u64_bytes_gen, u64, Vec<u8>, u64, ints(3000), bytes(5000));
roundtrip!(roundtrip_bytes_u64, roundtrip_bytes_u64_gen, Vec<u8>, u64, [u8], bytes(2000), ints(u64::MAX));

/// Damages the bytes of a written file and reads it back, checked.
macro_rules! damaged {
    ($fname:ident, $gname:ident, $k:ty, $v:ty, $q:ty, $keys:expr, $values:expr) => {
        pub fn $gname() -> BoxGen<DamagedCase<$k, $v>> {
            (gens::vec(($keys, $values)).len(0..=1_500), gens::vec(Damage::generator()).len(1..=4), gens::vec($keys).len(0..=6))
                .boxed()
        }

        pub fn $fname(case: &DamagedCase<$k, $v>) -> Result<(), Fail> {
            let (entries, damage, probes) = case;
            let model: BTreeMap<$k, $v> = entries.iter().cloned().collect();
            let path = TempPath::new();
            BBTreeMapFile::<$k, $v>::create(&path.0, &model).map_err(|e| fail("mmapdmg__error__create", format!("create: {e}")))?;
            let mut buf = fs::read(&path.0).unwrap();
            let mut len = buf.len();
            for d in damage {
                len = d.apply(&mut buf, len);
            }
            buf.truncate(len);
            fs::write(&path.0, &buf).unwrap();
            // `read` checks the file: it refuses or accepts, without panicking.
            let opened = catch_unwind(AssertUnwindSafe(|| BBTreeMapFile::<$k, $v>::read(&path.0)))
                .map_err(|_| fail("mmapdmg__panic__read", "BBTreeMapFile::read panicked on a damaged file"))?;
            if let Ok(file) = opened {
                classify(true, "opened");
                let bound = file.len().min(1 << 16) + 8;
                let _ = quietly(|| {
                    let mut seen = 0usize;
                    for k in probes {
                        let q: &$q = k.borrow_q();
                        seen += file.get(q).is_some() as usize;
                        seen += file.range::<$q, (Bound<&$q>, Bound<&$q>)>((Bound::Included(q), Bound::Unbounded)).take(4).count();
                        seen += file.range::<$q, (Bound<&$q>, Bound<&$q>)>((Bound::Unbounded, Bound::Excluded(q))).rev().take(4).count();
                    }
                    for (k, v) in file.iter().take(bound) {
                        seen += format!("{k:?}{v:?}").len();
                    }
                    seen += file.iter().rev().take(bound).count();
                    seen
                });
            }
            Ok(())
        }
    };
}

damaged!(damaged_bytes_bytes, damaged_bytes_bytes_gen, Vec<u8>, Vec<u8>, [u8], bytes(3000), bytes(6000));
damaged!(damaged_text_u64, damaged_text_u64_gen, String, u64, str, text(500), ints(u64::MAX));
damaged!(damaged_u64_text, damaged_u64_text_gen, u64, String, u64, ints(5000), text(2000));
