// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! `bbtree_store::wal`: a [`WalWriter`] on storage in memory that keeps S3
//! Express's rules (`design/bbtree-store/01-wal.md`, "Storage") and loses
//! requests and responses on demand, with other writers appending between
//! its storage calls and bytes damaged between operations. The storage's
//! files are the ground truth: a decoder of the documented layout
//! ("Layout") reads them, and every result is checked against them:
//!
//! - after a writer's operation the WAL still decodes (`wal__format__decode`)
//!   and no file takes more than `MAX_APPENDS` appends;
//! - an open replays every record but claims, in order, with its address,
//!   and writes exactly its claim; on corrupted bytes it fails with
//!   `Corrupted` naming the first bad file and offset ("Opening": "A failed
//!   check is corruption, an error, never the WAL's end");
//! - an append that returns `Ok` holds its records at the addresses it gives;
//!   `TailMoved` wrote nothing, `Storage` at most once ("never written
//!   twice"); a writer fenced by a later claim writes nothing ("A claim
//!   fences");
//! - `read_value` gives an insert's value, `Corrupted` for a removal.

use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::io;
use std::pin::pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll, Waker};

use bbtree_store::wal::{AppendError, Record, RecordAddress, WalError, WalStorage, WalWriter};
use ephyra::{BoxGen, GenExt, classify, gens};
use serde::{Deserialize, Serialize};

use crate::common::{Fail, ensure, fail, selftest};

/// The storage never waits: one poll finishes every future.
fn block_on<F: Future>(f: F) -> F::Output {
    let mut f = pin!(f);
    match f.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(v) => v,
        Poll::Pending => panic!("the in-memory WAL storage never waits"),
    }
}

// ---- the documented layout -------------------------------------------------

const HEADER: [u8; 12] = *b"bbtreeWL\x01\0\0\0";
const RECORD_HEADER: usize = 21;
const INSERT: u8 = 1;
const REMOVE: u8 = 2;
const CLAIM: u8 = 3;

#[derive(Clone, Debug, PartialEq)]
struct Decoded {
    file: u64,
    offset: u64,
    len: u64,
    lsn: u64,
    kind: u8,
    last: bool,
    key: Vec<u8>,
    value: Vec<u8>,
}

/// One record at the start of `bytes`: kind, last, lsn, key, value, length.
fn decode_one(bytes: &[u8]) -> Option<(u8, bool, u64, &[u8], &[u8], usize)> {
    if bytes.len() < RECORD_HEADER {
        return None;
    }
    let crc = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
    let lsn = u64::from_le_bytes(bytes[4..12].try_into().unwrap());
    let klen = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as u64;
    let vlen = u32::from_le_bytes(bytes[16..20].try_into().unwrap()) as u64;
    let (kind, last) = match bytes[20] {
        b @ (1..=3) => (b, false),
        b @ (0x81..=0x83) => (b & 0x7f, true),
        _ => return None,
    };
    let end = RECORD_HEADER as u64 + klen + vlen;
    if end > bytes.len() as u64 {
        return None;
    }
    let (end, kend) = (end as usize, RECORD_HEADER + klen as usize);
    if crc32c::crc32c(&bytes[4..end]) != crc {
        return None;
    }
    Some((kind, last, lsn, &bytes[RECORD_HEADER..kend], &bytes[kend..end], end))
}

fn encode_one(out: &mut Vec<u8>, lsn: u64, kind: u8, last: bool, key: &[u8], value: &[u8]) {
    let start = out.len();
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&lsn.to_le_bytes());
    out.extend_from_slice(&(key.len() as u32).to_le_bytes());
    out.extend_from_slice(&(value.len() as u32).to_le_bytes());
    out.push(kind | if last { 0x80 } else { 0 });
    out.extend_from_slice(key);
    out.extend_from_slice(value);
    let crc = crc32c::crc32c(&out[start + 4..]);
    out[start..start + 4].copy_from_slice(&crc.to_le_bytes());
}

/// Where the WAL ends: the last file, its length and appends, the next LSN.
#[derive(Clone, Copy, Debug, Default)]
struct Tail {
    file: u64,
    len: u64,
    appends: u64,
    next_lsn: u64,
}

#[derive(Debug)]
enum Scan {
    Ok(Vec<Decoded>, Tail),
    Corrupt { file: u64, offset: u64 },
}

/// "Opening": files 0, 1, … until one is missing; each a header, then
/// records whose LSNs go up by one, ending after an append's last record.
fn scan(files: &BTreeMap<u64, MemFile>) -> Scan {
    let mut records = Vec::new();
    let mut tail = Tail::default();
    let mut f = 0u64;
    while let Some(file) = files.get(&f) {
        let bytes = &file.bytes;
        if bytes.len() < HEADER.len() || bytes[..HEADER.len()] != HEADER {
            return Scan::Corrupt { file: f, offset: 0 };
        }
        let (mut pos, mut append_end, mut appends) = (HEADER.len(), HEADER.len(), 0);
        while pos < bytes.len() {
            let Some((kind, last, lsn, key, value, len)) = decode_one(&bytes[pos..]) else {
                return Scan::Corrupt { file: f, offset: pos as u64 };
            };
            if lsn != tail.next_lsn {
                return Scan::Corrupt { file: f, offset: pos as u64 };
            }
            records.push(Decoded { file: f, offset: pos as u64, len: len as u64, lsn, kind, last, key: key.to_vec(), value: value.to_vec() });
            pos += len;
            tail.next_lsn += 1;
            if last {
                appends += 1;
                append_end = pos;
            }
        }
        if append_end != pos {
            return Scan::Corrupt { file: f, offset: append_end as u64 };
        }
        tail = Tail { file: f, len: bytes.len() as u64, appends, next_lsn: tail.next_lsn };
        f += 1;
    }
    Scan::Ok(records, tail)
}

/// The appends a file's bytes hold whole, as a writer's scan counts them.
fn appends_in(bytes: &[u8]) -> u64 {
    let (mut pos, mut n) = (HEADER.len(), 0);
    while let Some((_, last, _, _, _, len)) = bytes.get(pos..).and_then(decode_one) {
        pos += len;
        n += u64::from(last);
    }
    n
}

/// A `RecordAddress`'s file, offset and length, from its `Debug` (its fields
/// are private).
fn address_parts(a: &RecordAddress) -> Option<(u64, u64, u64)> {
    let s = format!("{a:?}");
    let num = |name: &str| -> Option<u64> {
        let at = s.find(&format!("{name}: "))? + name.len() + 2;
        s[at..].chars().take_while(char::is_ascii_digit).collect::<String>().parse().ok()
    };
    Some((num("file")?, num("offset")?, num("len")?))
}

// ---- storage ---------------------------------------------------------------

#[derive(Clone, Default, Debug)]
struct MemFile {
    bytes: Vec<u8>,
    appends: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Fault {
    /// The append's outcome is unknown; it didn't happen.
    RequestLost,
    /// The append happened (written or refused); its response is lost.
    ResponseLost,
    /// No fault for this append.
    Fine,
}

/// Who wrote what: a storage append that took.
#[derive(Clone, Debug)]
struct Write {
    file: u64,
    offset: u64,
    len: u64,
    /// The operation in flight (`usize::MAX` for a rival).
    op: usize,
    /// It is an append of one claim.
    claim: bool,
}

/// Another writer, appending before the `calls`th next storage call.
#[derive(Clone, Debug)]
struct RivalAt {
    calls: u32,
    rival: Rival,
}

#[derive(Default)]
struct State {
    files: BTreeMap<u64, MemFile>,
    faults: VecDeque<Fault>,
    read_faults: u32,
    rival: Option<RivalAt>,
    writes: Vec<Write>,
    op: usize,
    /// An append past MAX_APPENDS was attempted at the right offset.
    over_max: Option<(u64, u64)>,
    rivals_done: u32,
}

#[derive(Clone, Default)]
pub struct Mem<const M: u32> {
    state: Arc<Mutex<State>>,
}

impl<const M: u32> Mem<M> {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl State {
    fn raw_append(&mut self, file: u64, offset: u64, bytes: &[u8], max: u32, op: usize) -> Result<(), AppendError> {
        let mismatch = match self.files.get(&file) {
            None => offset != 0,
            Some(f) => offset == 0 || f.bytes.len() as u64 != offset,
        };
        if mismatch {
            return Err(AppendError::OffsetMismatch);
        }
        let f = self.files.entry(file).or_default();
        if f.appends >= u64::from(max) {
            self.over_max.get_or_insert((file, offset));
            return Err(AppendError::OutcomeUnknown(io::Error::other("TooManyParts")));
        }
        f.bytes.extend_from_slice(bytes);
        f.appends += 1;
        let claim = matches!(decode_one(&bytes[if offset == 0 { HEADER.len().min(bytes.len()) } else { 0 }..]),
            Some((CLAIM, true, ..)));
        self.writes.push(Write { file, offset, len: bytes.len() as u64, op, claim });
        Ok(())
    }

    /// Runs a rival that is due before this storage call.
    fn tick(&mut self, max: u32) {
        let Some(r) = &mut self.rival else { return };
        if r.calls > 0 {
            r.calls -= 1;
            return;
        }
        let rival = self.rival.take().unwrap().rival;
        let Scan::Ok(_, mut tail) = scan(&self.files) else { return };
        if self.files.is_empty() {
            tail = Tail::default();
        }
        let mut appends: Vec<Vec<(u8, Vec<u8>, Vec<u8>)>> = Vec::new();
        if rival.claim {
            appends.push(vec![(CLAIM, rival.id.to_le_bytes().to_vec(), Vec::new())]);
        }
        for a in &rival.appends {
            if !a.is_empty() {
                appends.push(a.iter().map(Rec::parts).collect());
            }
        }
        for parts in appends {
            if tail.appends >= u64::from(max) {
                tail = Tail { file: tail.file + 1, len: 0, appends: 0, next_lsn: tail.next_lsn };
            }
            let mut bytes = Vec::new();
            if tail.len == 0 {
                bytes.extend_from_slice(&HEADER);
            }
            for (i, (kind, key, value)) in parts.iter().enumerate() {
                encode_one(&mut bytes, tail.next_lsn + i as u64, *kind, i + 1 == parts.len(), key, value);
            }
            if self.raw_append(tail.file, tail.len, &bytes, max, usize::MAX).is_err() {
                return;
            }
            tail = Tail { file: tail.file, len: tail.len + bytes.len() as u64, appends: tail.appends + 1, next_lsn: tail.next_lsn + parts.len() as u64 };
        }
        self.rivals_done += 1;
    }

    fn read_fault(&mut self) -> io::Result<()> {
        if self.read_faults > 0 {
            self.read_faults -= 1;
            return Err(io::Error::new(io::ErrorKind::TimedOut, "read lost"));
        }
        Ok(())
    }
}

impl<const M: u32> WalStorage for Mem<M> {
    const MAX_APPENDS: u32 = M;

    async fn append(&self, file: u64, offset: u64, bytes: &[u8]) -> Result<(), AppendError> {
        let mut st = self.lock();
        st.tick(M);
        let lost = || AppendError::OutcomeUnknown(io::Error::new(io::ErrorKind::TimedOut, "lost"));
        let op = st.op;
        match st.faults.pop_front().unwrap_or(Fault::Fine) {
            Fault::RequestLost => Err(lost()),
            Fault::ResponseLost => {
                let _ = st.raw_append(file, offset, bytes, M, op);
                Err(lost())
            }
            Fault::Fine => st.raw_append(file, offset, bytes, M, op),
        }
    }

    async fn len(&self, file: u64) -> io::Result<Option<u64>> {
        let mut st = self.lock();
        st.tick(M);
        st.read_fault()?;
        Ok(st.files.get(&file).map(|f| f.bytes.len() as u64))
    }

    async fn read(&self, file: u64, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        let mut st = self.lock();
        st.tick(M);
        st.read_fault()?;
        let f = st.files.get(&file).ok_or(io::ErrorKind::NotFound)?;
        let start = usize::try_from(offset).map_err(|_| io::ErrorKind::UnexpectedEof)?;
        let bytes = start.checked_add(buf.len()).and_then(|end| f.bytes.get(start..end)).ok_or(io::ErrorKind::UnexpectedEof)?;
        buf.copy_from_slice(bytes);
        Ok(())
    }
}

// ---- the case --------------------------------------------------------------

/// A value: its length and a fill seed, so that long values print short.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ValueSpec {
    len: u32,
    seed: u8,
}

impl ValueSpec {
    fn bytes(self) -> Vec<u8> {
        (0..self.len).map(|i| (i as u8).wrapping_mul(29).wrapping_add(self.seed)).collect()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Rec {
    Insert(Vec<u8>, ValueSpec),
    Remove(Vec<u8>),
}

impl Rec {
    fn parts(&self) -> (u8, Vec<u8>, Vec<u8>) {
        match self {
            Rec::Insert(k, v) => (INSERT, k.clone(), v.bytes()),
            Rec::Remove(k) => (REMOVE, k.clone(), Vec::new()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rival {
    /// It claims the WAL first, fencing the writers before it.
    claim: bool,
    id: u64,
    appends: Vec<Vec<Rec>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Damage {
    /// XORs a byte of a file (`at` scaled to its length).
    Flip { file: u8, at: u16, xor: u8 },
    /// Cuts a file to a length (scaled), as a torn append on a local disk
    /// would ("On a local SSD"); the writers open then retire.
    Truncate { file: u8, at: u16 },
    /// Bytes past the end of the last file.
    Garbage(Vec<u8>),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Op {
    /// A new writer opens the WAL.
    Open,
    /// The `writer`th newest open writer appends.
    Append { writer: u8, recs: Vec<Rec> },
    /// The next appends' faults, in order.
    Faults(Vec<Fault>),
    /// The next reads (and lengths) fail.
    ReadFaults(u8),
    /// Before the `calls`th next storage call, another writer appends.
    Rival { calls: u8, rival: Rival },
    Damage(Damage),
    /// `read_value` of every address handed out.
    ReadBack,
}

pub type Case = Vec<Op>;

fn key() -> BoxGen<Vec<u8>> {
    let alphabet = gens::pick_from(vec![0u8, 1, b'a', b'b', 0x80, 0xff]);
    gens::frequency(vec![
        (8, gens::vec(alphabet.clone()).len(0..=12).boxed()),
        (1, (gens::int(13usize..=600), gens::int(0u8..=255)).map(|(n, b)| vec![b; n]).boxed()),
    ])
    .boxed()
}

fn value() -> BoxGen<ValueSpec> {
    let len = gens::frequency(vec![
        (8, gens::int(0u32..=24).boxed()),
        (3, gens::int(0u32..=600).boxed()),
        (1, gens::int(600u32..=70_000).boxed()),
    ]);
    (len, gens::int(0u8..=255)).iso(|(len, seed)| ValueSpec { len, seed }, |v: &ValueSpec| (v.len, v.seed)).boxed()
}

fn rec() -> BoxGen<Rec> {
    gens::frequency(vec![(4, (key(), value()).map(|(k, v)| Rec::Insert(k, v)).boxed()), (1, key().map(Rec::Remove).boxed())]).boxed()
}

fn recs() -> BoxGen<Vec<Rec>> {
    gens::frequency(vec![
        (6, gens::vec(rec()).len(1..=1).boxed()),
        (3, gens::vec(rec()).len(0..=6).boxed()),
        (1, gens::vec(rec()).len(0..=40).boxed()),
    ])
    .boxed()
}

fn fault() -> BoxGen<Fault> {
    gens::frequency(vec![
        (2, gens::constant(Fault::RequestLost).boxed()),
        (2, gens::constant(Fault::ResponseLost).boxed()),
        (1, gens::constant(Fault::Fine).boxed()),
    ])
    .boxed()
}

fn damage() -> BoxGen<Damage> {
    gens::frequency(vec![
        (3, (gens::int(0u8..=255), gens::int(0u16..=u16::MAX), gens::int(1u8..=255)).map(|(file, at, xor)| Damage::Flip { file, at, xor }).boxed()),
        (2, (gens::int(0u8..=255), gens::int(0u16..=u16::MAX)).map(|(file, at)| Damage::Truncate { file, at }).boxed()),
        (1, gens::vec(gens::int(0u8..=255)).len(0..=40).map(Damage::Garbage).boxed()),
    ])
    .boxed()
}

fn op() -> BoxGen<Op> {
    let writer = gens::frequency(vec![(8, gens::constant(0u8).boxed()), (2, gens::int(0u8..=3).boxed())]);
    let rival = (gens::frequency(vec![(3, gens::constant(true).boxed()), (1, gens::constant(false).boxed())]), gens::int(0u64..=u64::MAX), gens::vec(recs()).len(0..=3))
        .map(|(claim, id, appends)| Rival { claim, id, appends });
    gens::frequency(vec![
        (3, gens::constant(Op::Open).boxed()),
        (14, (writer, recs()).map(|(writer, recs)| Op::Append { writer, recs }).boxed()),
        (3, gens::vec(fault()).len(1..=4).map(Op::Faults).boxed()),
        (1, gens::int(1u8..=3).map(Op::ReadFaults).boxed()),
        (2, (gens::int(0u8..=12), rival).map(|(calls, rival)| Op::Rival { calls, rival }).boxed()),
        (1, damage().map(Op::Damage).boxed()),
        (1, gens::constant(Op::ReadBack).boxed()),
    ])
    .boxed()
}

pub fn case() -> BoxGen<Case> {
    // `run` opens a writer first.
    gens::vec(op()).len(0..=80).boxed()
}

// ---- the run ---------------------------------------------------------------

struct Writer<S> {
    writer: WalWriter<S>,
    /// The index of its claim's write.
    claim_write: usize,
}

/// An address handed out, and what it must read.
struct Handed {
    address: RecordAddress,
    kind: u8,
    key: Vec<u8>,
    value: Vec<u8>,
}

fn err_name(e: &WalError) -> &'static str {
    match e {
        WalError::Storage(_) => "storage",
        WalError::TailMoved => "tail_moved",
        WalError::Corrupted { .. } => "corrupted",
    }
}

/// The record at an address, read straight from the files.
fn record_at(files: &BTreeMap<u64, MemFile>, file: u64, offset: u64, len: u64) -> Option<(u8, bool, u64, Vec<u8>, Vec<u8>)> {
    let bytes = &files.get(&file)?.bytes;
    let at = bytes.get(offset as usize..)?;
    let (kind, last, lsn, key, value, n) = decode_one(at)?;
    (n as u64 == len).then(|| (kind, last, lsn, key.to_vec(), value.to_vec()))
}

pub fn run<const M: u32>(case: &Case) -> Result<(), Fail> {
    let storage = Mem::<M>::default();
    let mut writers: Vec<Writer<Mem<M>>> = Vec::new();
    let mut handed: Vec<Handed> = Vec::new();
    // Damage the writers didn't make: the WAL may not decode.
    let mut damaged = false;
    // Claims before this write may have been damaged away.
    let mut fence_floor = 0;
    let first = Op::Open;
    let result = (|| {
    for (op_index, op) in std::iter::once(&first).chain(case).enumerate() {
        let writes_before = {
            let mut st = storage.lock();
            st.op = op_index;
            st.writes.len()
        };
        let ours = |st: &State| -> Vec<Write> { st.writes[writes_before..].iter().filter(|w| w.op == op_index).cloned().collect() };
        match op {
            Op::Open => {
                let before = scan(&storage.lock().files);
                let mut replayed: Vec<(RecordAddress, u8, Vec<u8>, Vec<u8>)> = Vec::new();
                let opened = block_on(WalWriter::open(storage.clone(), |address, record| {
                    replayed.push(match record {
                        Record::Insert { key, value } => (address, INSERT, key.to_vec(), value.to_vec()),
                        Record::Remove { key } => (address, REMOVE, key.to_vec(), Vec::new()),
                    })
                }));
                let st = storage.lock();
                let mine = ours(&st);
                let after = scan(&st.files);
                match (opened, &before) {
                    (Ok(_), Scan::Corrupt { file, offset }) => {
                        return Err(fail("wal__corrupt__accepted", format!("op {op_index}: an open succeeded on a WAL that fails its checks at file {file} offset {offset}")));
                    }
                    (Ok(writer), Scan::Ok(..)) => {
                        ensure(mine.len() == 1 && mine[0].claim, "wal__writes__open", || format!("op {op_index}: an open that succeeded wrote {mine:?}, not one claim"))?;
                        let claim = &mine[0];
                        let claim_write = st.writes.iter().rposition(|w| w.op == op_index).unwrap();
                        match &after {
                            Scan::Ok(records, _) => {
                                let claim_offset = claim.offset + if claim.offset == 0 { HEADER.len() as u64 } else { 0 };
                                let Some(claim_at) = records.iter().position(|r| r.file == claim.file && r.offset == claim_offset) else {
                                    return Err(fail("wal__writes__open", format!("op {op_index}: the open's claim {claim:?} isn't a record of the WAL")));
                                };
                                let got: Vec<(Option<(u64, u64, u64)>, u8, &[u8], &[u8])> = replayed.iter().map(|(a, k, key, v)| (address_parts(a), *k, &key[..], &v[..])).collect();
                                let want: Vec<(Option<(u64, u64, u64)>, u8, &[u8], &[u8])> =
                                    records[..claim_at].iter().filter(|r| r.kind != CLAIM).map(|r| (Some((r.file, r.offset, r.len)), r.kind, &r.key[..], &r.value[..])).collect();
                                if got != want {
                                    let first = got.iter().zip(&want).position(|(a, b)| a != b).unwrap_or(got.len().min(want.len()));
                                    return Err(fail(
                                        "wal__mismatch__replay",
                                        format!(
                                            "op {op_index}: the open replayed {} records, the WAL holds {} before its claim; first difference at {first}: got {:.300} want {:.300}",
                                            got.len(),
                                            want.len(),
                                            format!("{:?}", got.get(first)),
                                            format!("{:?}", want.get(first))
                                        ),
                                    ));
                                }
                                if damaged {
                                    // The damage left a WAL that passes: what it holds now is what counts.
                                    damaged = false;
                                    handed.retain(|h| {
                                        address_parts(&h.address).is_some_and(|(f, o, l)| record_at(&st.files, f, o, l).is_some_and(|(k, _, _, key, value)| k == h.kind && key == h.key && value == h.value))
                                    });
                                }
                                for (address, kind, key, value) in replayed {
                                    handed.push(Handed { address, kind, key, value });
                                }
                            }
                            Scan::Corrupt { file, offset } => {
                                // A file left past a dropped one, now after the claim: only damage does that.
                                ensure(damaged, "wal__format__decode", || format!("op {op_index}: after an open the WAL fails its checks at file {file} offset {offset}"))?;
                            }
                        }
                        classify(claim.file > 0, "multi_file");
                        writers.push(Writer { writer, claim_write });
                        if writers.len() > 6 {
                            writers.remove(0);
                        }
                    }
                    (Err(e), _) => {
                        classify(true, &format!("open_{}", err_name(&e)));
                        match (&e, &before) {
                            (WalError::Corrupted { file, offset }, Scan::Corrupt { file: f, offset: o }) => {
                                ensure(mine.is_empty(), "wal__writes__open", || format!("op {op_index}: an open that found corruption wrote {mine:?}"))?;
                                ensure((file, offset) == (f, o), "wal__corrupt__offset", || format!("op {op_index}: the open names file {file} offset {offset}; the first failed check is at file {f} offset {o}"))?;
                            }
                            (WalError::Corrupted { file, offset }, Scan::Ok(..)) => {
                                return Err(fail("wal__corrupt__false_alarm", format!("op {op_index}: the open says file {file} is corrupted at {offset}; the WAL passes every check")));
                            }
                            (WalError::Storage(_), _) => {
                                ensure(mine.len() <= 1 && mine.iter().all(|w| w.claim), "wal__writes__open", || format!("op {op_index}: an open that failed in storage wrote {mine:?}"))?;
                            }
                            (WalError::TailMoved, _) => {
                                return Err(fail("wal__result__open_tail_moved", format!("op {op_index}: an open failed with TailMoved; it scans on and claims again")));
                            }
                        }
                        if let (false, Scan::Corrupt { file, offset }) = (damaged, &after) {
                            return Err(fail("wal__format__decode", format!("op {op_index}: after an open the WAL fails its checks at file {file} offset {offset}")));
                        }
                    }
                }
            }
            Op::Append { writer, recs } => {
                if writers.is_empty() {
                    continue;
                }
                let n = writers.len();
                let w = &mut writers[n - 1 - (*writer as usize % n)];
                let parts: Vec<(u8, Vec<u8>, Vec<u8>)> = recs.iter().map(Rec::parts).collect();
                let records: Vec<Record<'_>> = parts
                    .iter()
                    .map(|(kind, key, value)| if *kind == INSERT { Record::Insert { key, value } } else { Record::Remove { key } })
                    .collect();
                let result = block_on(w.writer.append(&records));
                let st = storage.lock();
                let mine = ours(&st);
                // Fenced: a claim written after this writer's (and after the last damage, which may have dropped one).
                let mut fence = None;
                for (i, x) in st.writes.iter().enumerate().skip((w.claim_write + 1).max(fence_floor)) {
                    if x.op == op_index {
                        if let Some(f) = fence {
                            return Err(fail("wal__fence__append", format!("op {op_index}: a writer fenced by {:?} wrote {x:?}", st.writes[f])));
                        }
                    } else if x.claim {
                        fence.get_or_insert(i);
                    }
                }
                match &result {
                    Ok(addresses) => {
                        ensure(mine.len() <= 1, "wal__writes__append", || format!("op {op_index}: one append wrote {mine:?}"))?;
                        ensure(addresses.len() == parts.len(), "wal__mismatch__address", || format!("op {op_index}: {} records, {} addresses", parts.len(), addresses.len()))?;
                        let mut lsn = None;
                        for (i, (address, (kind, key, value))) in addresses.iter().zip(&parts).enumerate() {
                            let at = address_parts(address).and_then(|(f, o, l)| record_at(&st.files, f, o, l));
                            let ok = match &at {
                                Some((k, last, l, ak, av)) => {
                                    let next = lsn.is_none_or(|p: u64| *l == p + 1);
                                    lsn = Some(*l);
                                    k == kind && ak == key && av == value && *last == (i + 1 == parts.len()) && next
                                }
                                None => false,
                            };
                            ensure(ok, "wal__mismatch__address", || {
                                format!("op {op_index}: append Ok, record {i} of {} ({kind}, key {} bytes, value {} bytes) at {address:?} reads {:.200}", parts.len(), key.len(), value.len(), format!("{at:?}"))
                            })?;
                        }
                        for (address, (kind, key, value)) in addresses.iter().zip(parts) {
                            handed.push(Handed { address: *address, kind, key, value });
                        }
                    }
                    Err(e) => {
                        classify(true, &format!("append_{}", err_name(e)));
                        match e {
                            WalError::TailMoved => {
                                ensure(mine.is_empty(), "wal__writes__append", || format!("op {op_index}: an append that failed with TailMoved wrote {mine:?}"))?;
                            }
                            WalError::Storage(_) => {
                                ensure(mine.len() <= 1, "wal__writes__append", || format!("op {op_index}: an append that failed in storage wrote {mine:?}"))?;
                            }
                            WalError::Corrupted { file, offset } => {
                                return Err(fail("wal__result__append_corrupted", format!("op {op_index}: an append failed with Corrupted {{ file: {file}, offset: {offset} }}")));
                            }
                        }
                    }
                }
                if !damaged {
                    if let Scan::Corrupt { file, offset } = scan(&st.files) {
                        return Err(fail("wal__format__decode", format!("op {op_index}: after an append ({:?}) the WAL fails its checks at file {file} offset {offset}", result.as_ref().map(|a| a.len()).map_err(err_name))));
                    }
                }
            }
            Op::Faults(faults) => storage.lock().faults.extend(faults.iter().copied()),
            Op::ReadFaults(n) => storage.lock().read_faults = u32::from(*n),
            Op::Rival { calls, rival } => {
                classify(true, "rival");
                storage.lock().rival = Some(RivalAt { calls: u32::from(*calls), rival: rival.clone() });
            }
            Op::Damage(d) => {
                let mut st = storage.lock();
                let last = st.files.keys().next_back().copied();
                let Some(last) = last else { continue };
                let pick = |n: u8| u64::from(n) % (last + 1);
                match d {
                    Damage::Flip { file, at, xor } => {
                        if let Some(f) = st.files.get_mut(&pick(*file)).filter(|f| !f.bytes.is_empty()) {
                            let i = *at as usize * f.bytes.len() / (u16::MAX as usize + 1);
                            f.bytes[i] ^= xor;
                        }
                    }
                    Damage::Truncate { file, at } => {
                        if let Some(f) = st.files.get_mut(&pick(*file)) {
                            let n = *at as usize * f.bytes.len() / (u16::MAX as usize + 1);
                            f.bytes.truncate(n);
                            // Storage only appends: a writer would append where it already
                            // wrote. What it means for writers is out of scope; they retire.
                            writers.clear();
                        }
                    }
                    Damage::Garbage(bytes) => st.files.get_mut(&last).unwrap().bytes.extend_from_slice(bytes),
                }
                for f in st.files.values_mut() {
                    f.appends = appends_in(&f.bytes);
                }
                damaged = true;
                fence_floor = st.writes.len();
                st.over_max = None;
                classify(true, "damaged");
            }
            Op::ReadBack => {
                if damaged {
                    continue;
                }
                for h in &handed {
                    let read = block_on(h.address.read_value(&storage));
                    match (&read, h.kind) {
                        (Err(WalError::Storage(_)), _) => {}
                        (Ok(v), INSERT) if *v == h.value => {}
                        (Err(WalError::Corrupted { file, offset }), REMOVE) => {
                            let parts = address_parts(&h.address);
                            ensure(parts.is_some_and(|(f, o, _)| (f, o) == (*file, *offset)), "wal__mismatch__read_value", || format!("a removal at {:?} reads Corrupted at file {file} offset {offset}", h.address))?;
                        }
                        _ => {
                            return Err(fail(
                                "wal__mismatch__read_value",
                                format!("read_value at {:?} of a record of kind {} (key {} bytes, value {} bytes) gives {:.200}", h.address, h.kind, h.key.len(), h.value.len(), format!("{:?}", read.as_ref().map(|v| v.len()).map_err(err_name))),
                            ));
                        }
                    }
                }
            }
        }
        let st = storage.lock();
        if let Some((file, offset)) = st.over_max {
            return Err(fail("wal__contract__max_appends", format!("op {op_index}: an append to file {file} at its end (offset {offset}) past its {M} appends")));
        }
    }
    Ok(())
    })();
    if let Err(mut f) = result {
        if std::env::var("BBZ_WAL_DEBUG").is_ok() {
            let st = storage.lock();
            f.msg.push_str(&format!("\nwrites: {:?}", st.writes));
            for (n, file) in &st.files {
                let mut pos = HEADER.len();
                f.msg.push_str(&format!("\nfile {n} ({} bytes, {} appends):", file.bytes.len(), file.appends));
                while let Some((kind, last, lsn, _, _, len)) = file.bytes.get(pos..).and_then(decode_one) {
                    f.msg.push_str(&format!(" [{pos} lsn {lsn} k{kind}{}]", if last { "!" } else { "" }));
                    pos += len;
                }
            }
        }
        return Err(f);
    }
    let st = storage.lock();
    classify(st.rivals_done > 0, "rival_ran");
    if selftest("break") && st.files.len() > 1 {
        return Err(fail("wal__selftest__break", "self-test"));
    }
    Ok(())
}
