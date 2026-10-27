// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! `bbtree-mmap`: a `BBTreeMapFile` written from a map and opened every way
//! (read whole, mapped, embedded at an offset in a larger file) answers as
//! std's `BTreeMap` does; and `BBTreeMapFile::read`, which checks the file,
//! on damaged bytes refuses them or gives a map that may answer wrongly or
//! panic, never anything worse (AddressSanitizer for the "worse").

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::ops::Bound;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use bbtree_properties::{Damage, Watchdog, range_panics};
use bbtree_mmap::BBTreeMapFile;
use ephyra::{BoxGen, Config, GenExt, Report, TestResult, check, classify, gens};

static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

/// A file in the temp directory, removed when dropped.
struct TempPath(PathBuf);

impl TempPath {
    fn new() -> Self {
        let n = NEXT_FILE.fetch_add(1, Ordering::Relaxed);
        TempPath(std::env::temp_dir().join(format!("bbtree-harness-{}-{n}", std::process::id())))
    }
}

impl Drop for TempPath {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Byte strings from a small alphabet, sharing prefixes, a few long.
fn bytes(long: usize) -> BoxGen<Vec<u8>> {
    let alphabet = gens::pick_from(vec![0u8, 1, b'a', b'b', 0x7f, 0xff]);
    gens::frequency(vec![
        (6, gens::vec(alphabet.clone()).len(0..=10).boxed()),
        (2, gens::vec(alphabet.clone()).len(0..=40).boxed()),
        (1, (gens::vec(alphabet).len(0..=3), gens::int(0usize..=long), gens::int(0u8..=255))
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

/// Text: the byte strings' alphabet as chars, some multi-byte.
fn text(long: usize) -> BoxGen<String> {
    let chars = gens::pick_from(vec!['\0', 'a', 'b', 'é', '\u{7f}', '\u{10FFFF}', '中']);
    gens::frequency(vec![
        (6, gens::string_len(chars.clone(), 0..=10).boxed()),
        (1, gens::string_len(chars, 0..=long).boxed()),
    ])
    .boxed()
}

#[derive(Clone, Debug, PartialEq)]
enum Probe<K> {
    Get(K),
    Range(Bound<K>, Bound<K>, Vec<bool>),
}

fn probes<K: Clone + std::fmt::Debug + PartialEq + Send + Sync + 'static>(key: BoxGen<K>) -> BoxGen<Vec<Probe<K>>> {
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

/// Opens the file of `model` every way and checks each against it.
/// `$view` turns a std entry into what a lookup hands back.
macro_rules! roundtrip {
    ($name:ident, $k:ty, $v:ty, $q:ty, $keys:expr, $values:expr) => {
        #[test]
        fn $name() {
            let generator = (gens::vec(($keys, $values)).len(0..=2_000), probes($keys), gens::int(0u64..=5000));
            print(check(&config(stringify!($name)), generator, |(entries, probes, offset)| -> TestResult {
                let model: BTreeMap<$k, $v> = entries.into_iter().collect();
                let whole = TempPath::new();
                if let Err(e) = BBTreeMapFile::<$k, $v>::create(&whole.0, &model) {
                    return TestResult::Fail(format!("create: {e}"));
                }
                // The same map embedded after `offset` bytes of other data,
                // with more data after it that write_at cuts.
                let embedded = TempPath::new();
                let range = {
                    let mut file = OpenOptions::new().read(true).write(true).create(true).truncate(true).open(&embedded.0).unwrap();
                    file.write_all(&vec![0xEE; offset as usize + 100]).unwrap();
                    match BBTreeMapFile::<$k, $v>::write_at(&mut file, offset, &model) {
                        Ok(range) => range,
                        Err(e) => return TestResult::Fail(format!("write_at: {e}")),
                    }
                };
                if range.start < offset {
                    return TestResult::Fail(format!("write_at returned {range:?} before the offset {offset}"));
                }
                let prefix_kept = fs::read(&embedded.0).unwrap()[..offset as usize].iter().all(|&b| b == 0xEE);
                if !prefix_kept {
                    return TestResult::Fail("write_at changed bytes before its offset".into());
                }
                let opened: Vec<(&str, std::io::Result<BBTreeMapFile<$k, $v>>)> = vec![
                    ("read", BBTreeMapFile::<$k, $v>::read(&whole.0)),
                    ("mmap", unsafe { BBTreeMapFile::<$k, $v>::mmap(&whole.0).open() }),
                    ("read_range", BBTreeMapFile::<$k, $v>::read_range(&mut File::open(&embedded.0).unwrap(), range.clone())),
                    ("mmap range", unsafe { BBTreeMapFile::<$k, $v>::mmap(&embedded.0).range(range.clone()).open() }),
                ];
                for (how, file) in opened {
                    let file = match file {
                        Ok(file) => file,
                        Err(e) => return TestResult::Fail(format!("{how}: open failed: {e}")),
                    };
                    if file.len() != model.len() {
                        return TestResult::Fail(format!("{how}: len {} vs {}", file.len(), model.len()));
                    }
                    let want: Vec<_> = model.iter().map(|(k, v)| (view_k(k), view_v(v))).collect();
                    let got: Vec<_> = file.iter().collect();
                    if got != want {
                        return TestResult::Fail(format!("{how}: iter differs ({} vs {} entries)", got.len(), want.len()));
                    }
                    let got_rev: Vec<_> = file.iter().rev().collect();
                    if got_rev.iter().rev().ne(want.iter()) {
                        return TestResult::Fail(format!("{how}: iter().rev() differs"));
                    }
                    for (k, v) in model.iter().step_by((model.len() / 32).max(1)) {
                        let q: &$q = k.borrow_q();
                        if file.get(q) != Some(view_v(v)) {
                            return TestResult::Fail(format!("{how}: get({k:?}) {:?}", file.get(q)));
                        }
                    }
                    for probe in &probes {
                        match probe {
                            Probe::Get(k) => {
                                let q: &$q = k.borrow_q();
                                let (got, want) = (file.get(q), model.get(k).map(view_v));
                                if got != want {
                                    return TestResult::Fail(format!("{how}: get({k:?}) {got:?} vs {want:?}"));
                                }
                                let (got, want) = (file.get_key_value(q), model.get_key_value(k).map(|(k, v)| (view_k(k), view_v(v))));
                                if got != want {
                                    return TestResult::Fail(format!("{how}: get_key_value({k:?}) {got:?} vs {want:?}"));
                                }
                            }
                            Probe::Range(s, e, walk) => {
                                                                if range_panics(s, e) {
                                    let ours = catch_unwind(AssertUnwindSafe(|| file.range::<$q, _>((qb::<$k, $q>(s), qb::<$k, $q>(e))).count())).is_err();
                                    let std = !model.is_empty();
                                    if ours != std && !model.is_empty() {
                                        return TestResult::Fail(format!("{how}: range({s:?}, {e:?}) panics {ours}, std {std}"));
                                    }
                                    continue;
                                }
                                let mut ours = file.range::<$q, _>((qb::<$k, $q>(s), qb::<$k, $q>(e)));
                                let mut want = model.range((s.clone(), e.clone())).map(|(k, v)| (view_k(k), view_v(v)));
                                for (i, &front) in walk.iter().enumerate() {
                                    let (a, b) = if front { (ours.next(), want.next()) } else { (ours.next_back(), want.next_back()) };
                                    if a != b {
                                        return TestResult::Fail(format!("{how}: range({s:?}, {e:?}) step {i}: {a:?} vs {b:?}"));
                                    }
                                }
                                if ours.ne(want) {
                                    return TestResult::Fail(format!("{how}: range({s:?}, {e:?}) differs after the walk"));
                                }
                            }
                        }
                    }
                }
                TestResult::Pass
            }))
            .assert_passed();
        }
    };
}

/// A bound on keys as a bound on the query type.
fn qb<K: BorrowQ<Q>, Q: ?Sized>(b: &Bound<K>) -> Bound<&Q> {
    match b {
        Bound::Included(k) => Bound::Included(k.borrow_q()),
        Bound::Excluded(k) => Bound::Excluded(k.borrow_q()),
        Bound::Unbounded => Bound::Unbounded,
    }
}

/// A key as the query type a lookup takes.
trait BorrowQ<Q: ?Sized> {
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
trait View {
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
fn view_k<T: View>(t: &T) -> T::Out<'_> {
    t.view()
}
fn view_v<T: View>(t: &T) -> T::Out<'_> {
    t.view()
}

fn config(name: &str) -> Config {
    Config::default().name(name).tests(200)
}

fn print(report: Report) -> Report {
    eprintln!("{report}");
    report
}

roundtrip!(mmap_bytes_bytes, Vec<u8>, Vec<u8>, [u8], bytes(3000), bytes(9000));
roundtrip!(mmap_text_text, String, String, str, text(1500), text(3000));
roundtrip!(mmap_u64_bytes, u64, Vec<u8>, u64, gens::int(0u64..=3000).boxed(), bytes(5000));
roundtrip!(mmap_bytes_u64, Vec<u8>, u64, [u8], bytes(2000), gens::int(0u64..=u64::MAX).boxed());

/// Damages the bytes of a written file and reads it back, checked.
macro_rules! damaged {
    ($name:ident, $k:ty, $v:ty, $q:ty, $keys:expr, $values:expr) => {
        #[test]
        fn $name() {
            let watchdog = Watchdog::new(stringify!($name), std::time::Duration::from_secs(10));
            let generator = (gens::vec(($keys, $values)).len(0..=1_500), gens::vec(Damage::generator()).len(1..=4), gens::vec($keys).len(0..=6));
            print(check(&config(stringify!($name)), generator, |input| -> TestResult {
                watchdog.run(&input, || {
                    let (entries, damage, probes) = &input;
                    let model: BTreeMap<$k, $v> = entries.iter().cloned().collect();
                    let path = TempPath::new();
                    BBTreeMapFile::<$k, $v>::create(&path.0, &model).expect("create");
                    let mut buf = fs::read(&path.0).unwrap();
                    let mut len = buf.len();
                    for d in damage {
                        len = d.apply(&mut buf, len);
                    }
                    buf.truncate(len);
                    fs::write(&path.0, &buf).unwrap();
                    if let Ok(file) = BBTreeMapFile::<$k, $v>::read(&path.0) {
                        classify(true, "opened");
                        let bound = file.len().min(1 << 16) + 8;
                        let _ = catch_unwind(AssertUnwindSafe(|| {
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
                        }));
                    }
                    TestResult::Pass
                })
            }))
            .assert_passed();
        }
    };
}

damaged!(damaged_mmap_bytes_bytes, Vec<u8>, Vec<u8>, [u8], bytes(3000), bytes(6000));
damaged!(damaged_mmap_text_u64, String, u64, str, text(500), gens::int(0u64..=u64::MAX).boxed());
damaged!(damaged_mmap_u64_text, u64, String, u64, gens::int(0u64..=5000).boxed(), text(2000));
