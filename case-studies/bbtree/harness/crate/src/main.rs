// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! bbz: the bbtree harness for ephyra-agent (README.md).
//!
//!   bbz search <seed> <secs> <out>     supervisor: workers, findings, stats.json
//!   bbz worker <seed> <deadline-ms> <inflight>   (started by search)
//!   bbz repro <case.json>              exit 1 = reproduces, 0 = not, 2 = undecided
//!   bbz run-case <case.json>           one case in this process (repro's child)
//!   bbz gen <property> <seed> <n>      samples of a property's inputs
//!   bbz list                           the properties, weights and chunk sizes

mod annex;
mod bulk;
mod chaos;
mod common;
mod filemap;
mod mmap;
mod mmapmx;
mod pagemx;
mod store;
mod storelin;
mod storethreads;
mod syncalloc;
mod syncpanic;
mod synclin;
mod syncthreads;
mod sametree;
mod shlin;
mod ukey;
mod umodel;
mod upanic;
mod wal;
mod zkey;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt::Debug;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::os::unix::fs::PermissionsExt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bbtree::Auto;
use ephyra::{BoxGen, Config, Gen, Source, TestResult, check};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use common::Fail;
use filemap::{BigInternals, BigLeaves, Cfg, CfgInline, TinyInternals};

/// What a chunk of one property reports to the supervisor.
struct ChunkOut {
    cases: u64,
    labels: BTreeMap<String, u64>,
    /// Signature, message, case (`{"property", "input"}`).
    finding: Option<(String, String, Value)>,
}

/// The worker's side: where the case in flight is written before it runs.
struct Ctx {
    inflight: File,
    deadline: Instant,
}

impl Ctx {
    fn inflight(&mut self, property: &str, input: &impl Serialize) {
        let bytes = serde_json::to_vec(&json!({"property": property, "input": input})).unwrap_or_default();
        let _ = self.inflight.seek(SeekFrom::Start(0));
        let _ = self.inflight.set_len(0);
        let _ = self.inflight.write_all(&bytes);
    }
}

struct Prop {
    name: &'static str,
    /// The oracle: the first part of every signature this property gives.
    family: &'static str,
    weight: u32,
    /// Tests per chunk.
    tests: u64,
    /// Repro attempts (nondeterministic properties need several).
    attempts: u32,
    chunk: fn(&mut Ctx, u64, u64) -> ChunkOut,
    run_json: fn(&Value) -> Result<Result<(), Fail>, String>,
    samples: fn(u64, usize),
}

/// Runs `run` on `input`, turning an escaping panic into a crash failure.
fn guard(family: &str, run: impl FnOnce() -> Result<(), Fail>) -> Result<(), Fail> {
    match catch_unwind(AssertUnwindSafe(run)) {
        Ok(r) => r,
        Err(p) => {
            let msg = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()));
            Err(common::fail(&format!("crash__panic__{family}"), format!("panicked: {}", msg.unwrap_or_default())))
        }
    }
}

/// One chunk: an ephyra run of `tests` cases. The first failure fixes the
/// signature; while shrinking, failures of another signature are
/// discarded, so the minimal case keeps it. The last failing input is the
/// minimal one.
fn chunk<I>(ctx: &mut Ctx, name: &'static str, family: &'static str, generator: BoxGen<I>, run: fn(&I) -> Result<(), Fail>, seed: u64, tests: u64) -> ChunkOut
where
    I: Clone + Debug + Serialize + 'static,
{
    let left = ctx.deadline.saturating_duration_since(Instant::now());
    let mut cfg = Config::default().name(name).tests(tests).seed(seed).budget(left.min(Duration::from_secs(4)).max(Duration::from_millis(50)));
    cfg.regressions.save = false;
    cfg.regressions.replay = false;
    cfg.shrink.max_steps = 2000;
    cfg.shrink.timeout_ms = Some(left.min(Duration::from_secs(20)).as_millis() as u64);
    cfg.coverage.enabled = false;
    cfg.report.samples = 0;
    let first_sig: RefCell<Option<String>> = RefCell::new(None);
    let last: RefCell<Option<(I, Fail)>> = RefCell::new(None);
    let report = check(&cfg, generator, |input: I| {
        ctx.inflight(name, &input);
        match guard(family, || run(&input)) {
            Ok(()) => TestResult::Pass,
            Err(f) => {
                let mut sig = first_sig.borrow_mut();
                match &*sig {
                    None => *sig = Some(f.sig.clone()),
                    Some(s) if *s != f.sig => return TestResult::Discard("another signature".into()),
                    _ => {}
                }
                let msg = f.msg.clone();
                *last.borrow_mut() = Some((input, f));
                TestResult::Fail(msg)
            }
        }
    });
    let finding = if report.passed() {
        None
    } else if let Some((input, f)) = last.into_inner() {
        Some((f.sig, f.msg, json!({"property": name, "input": input})))
    } else {
        Some((format!("harness__error__{family}"), format!("{:?}: {:.1500}", report.status, format!("{:?}", report.failure)), json!({"property": name, "input": null})))
    };
    ChunkOut { cases: report.tests + u64::from(!report.passed()), labels: report.labels.clone(), finding }
}

fn run_json<I: DeserializeOwned>(v: &Value, family: &str, run: fn(&I) -> Result<(), Fail>) -> Result<Result<(), Fail>, String> {
    let input: I = serde_json::from_value(v.clone()).map_err(|e| e.to_string())?;
    Ok(guard(family, || run(&input)))
}

fn samples<I: Debug>(generator: BoxGen<I>, seed: u64, n: usize) {
    for i in 0..n {
        let mut src = Source::new(seed.wrapping_add(i as u64), (i * 100 / n.max(1)).max(1));
        println!("{:?}", generator.generate(&mut src));
    }
}

macro_rules! p {
    ($name:literal, $family:literal, $w:expr, $tests:expr, $att:expr, $ty:ty, $gen:expr, $run:expr) => {
        Prop {
            name: $name,
            family: $family,
            weight: $w,
            tests: $tests,
            attempts: $att,
            chunk: |ctx, seed, tests| chunk::<$ty>(ctx, $name, $family, $gen, $run, seed, tests),
            run_json: |v| run_json::<$ty>(v, $family, $run),
            samples: |seed, n| samples::<$ty>($gen, seed, n),
        }
    };
}

fn registry() -> Vec<Prop> {
    use filemap::{DamagedCase as FD, RoundtripCase as FR, damaged as fd, damaged_gen as fdg, roundtrip as fr, roundtrip_gen as frg};
    vec![
        // bbtree::file round trips: Auto's inline-key types, whole-compare
        // keys, other node configs and strategies.
        p!("filemap_rt_u64_u64", "filemap", 2, 200, 1, FR, frg(), fr::<u64, u64, Auto>),
        p!("filemap_rt_i64_u8", "filemap", 2, 200, 1, FR, frg(), fr::<i64, u8, Auto>),
        p!("filemap_rt_u128_u32", "filemap", 2, 200, 1, FR, frg(), fr::<u128, u32, Auto>),
        p!("filemap_rt_i128_i128", "filemap", 2, 200, 1, FR, frg(), fr::<i128, i128, Auto>),
        p!("filemap_rt_arr16_u64", "filemap", 2, 200, 1, FR, frg(), fr::<[u8; 16], u64, Auto>),
        p!("filemap_rt_arr20_arr3", "filemap", 2, 200, 1, FR, frg(), fr::<[u8; 20], [u8; 3], Auto>),
        p!("filemap_rt_arr32_u16", "filemap", 2, 200, 1, FR, frg(), fr::<[u8; 32], u16, Auto>),
        p!("filemap_rt_u32_u64", "filemap", 2, 200, 1, FR, frg(), fr::<u32, u64, Auto>),
        p!("filemap_rt_u8_arr64", "filemap", 2, 200, 1, FR, frg(), fr::<u8, [u8; 64], Auto>),
        p!("filemap_rt_i16_arr0", "filemap", 2, 200, 1, FR, frg(), fr::<i16, [u8; 0], Auto>),
        p!("filemap_rt_arr7_arr200", "filemap", 2, 200, 1, FR, frg(), fr::<[u8; 7], [u8; 200], Auto>),
        p!("filemap_rt_cfg_big_leaves_u64", "filemap", 2, 200, 1, FR, frg(), fr::<u64, u64, Cfg<BigLeaves>>),
        p!("filemap_rt_cfg_tiny_u64_u32", "filemap", 2, 200, 1, FR, frg(), fr::<u64, u32, Cfg<TinyInternals>>),
        p!("filemap_rt_cfg_tiny_u16_u64", "filemap", 2, 200, 1, FR, frg(), fr::<u16, u64, Cfg<TinyInternals>>),
        p!("filemap_rt_cfg_big_int_arr20_u8", "filemap", 2, 200, 1, FR, frg(), fr::<[u8; 20], u8, Cfg<BigInternals>>),
        p!("filemap_rt_cfg_big_int_u32_arr40", "filemap", 2, 200, 1, FR, frg(), fr::<u32, [u8; 40], Cfg<BigInternals>>),
        p!("filemap_rt_inline_u64", "filemap", 2, 200, 1, FR, frg(), fr::<u64, u64, bbtree::WithInlineKey>),
        p!("filemap_rt_inline_tiny_arr32_u64", "filemap", 2, 200, 1, FR, frg(), fr::<[u8; 32], u64, CfgInline<TinyInternals>>),
        // FileMap::open on damaged bytes (panics after open allowed).
        p!("filemapdmg_u64_u64", "filemapdmg", 8, 1000, 1, FD, fdg(), fd::<u64, u64>),
        p!("filemapdmg_arr20_u32", "filemapdmg", 8, 1000, 1, FD, fdg(), fd::<[u8; 20], u32>),
        p!("filemapdmg_u16_u8", "filemapdmg", 8, 1000, 1, FD, fdg(), fd::<u16, u8>),
        p!("filemapdmg_i128_arr24", "filemapdmg", 8, 1000, 1, FD, fdg(), fd::<i128, [u8; 24]>),
        // bbtree-mmap
        p!("mmap_rt_bytes_bytes", "mmap", 4, 30, 1, mmap::RoundtripCase<Vec<u8>, Vec<u8>>, mmap::roundtrip_bytes_bytes_gen(), mmap::roundtrip_bytes_bytes),
        p!("mmap_rt_text_text", "mmap", 4, 30, 1, mmap::RoundtripCase<String, String>, mmap::roundtrip_text_text_gen(), mmap::roundtrip_text_text),
        p!("mmap_rt_u64_bytes", "mmap", 4, 30, 1, mmap::RoundtripCase<u64, Vec<u8>>, mmap::roundtrip_u64_bytes_gen(), mmap::roundtrip_u64_bytes),
        p!("mmap_rt_bytes_u64", "mmap", 4, 30, 1, mmap::RoundtripCase<Vec<u8>, u64>, mmap::roundtrip_bytes_u64_gen(), mmap::roundtrip_bytes_u64),
        p!("mmapdmg_bytes_bytes", "mmapdmg", 6, 60, 1, mmap::DamagedCase<Vec<u8>, Vec<u8>>, mmap::damaged_bytes_bytes_gen(), mmap::damaged_bytes_bytes),
        p!("mmapdmg_text_u64", "mmapdmg", 6, 60, 1, mmap::DamagedCase<String, u64>, mmap::damaged_text_u64_gen(), mmap::damaged_text_u64),
        p!("mmapdmg_u64_text", "mmapdmg", 6, 60, 1, mmap::DamagedCase<u64, String>, mmap::damaged_u64_text_gen(), mmap::damaged_u64_text),
        // mmapmx: plain key types, value types and input paths for BBTreeMapFile / FileWriter
        p!("mmapmx_arr3_u8", "mmapmx", 4, 150, 1, mmapmx::MxCase, mmapmx::gen_case(), mmapmx::run_file::<[u8; 3], u8, false>),
        p!("mmapmx_arr16_f64", "mmapmx", 4, 150, 1, mmapmx::MxCase, mmapmx::gen_case(), mmapmx::run_file::<[u8; 16], f64, true>),
        p!("mmapmx_arr20_arr5", "mmapmx", 4, 150, 1, mmapmx::MxCase, mmapmx::gen_case(), mmapmx::run_file::<[u8; 20], [u8; 5], true>),
        p!("mmapmx_arr32_zst", "mmapmx", 4, 150, 1, mmapmx::MxCase, mmapmx::gen_case(), mmapmx::run_file::<[u8; 32], [u8; 0], true>),
        p!("mmapmx_arr70_u128", "mmapmx", 4, 150, 1, mmapmx::MxCase, mmapmx::gen_case(), mmapmx::run_file::<[u8; 70], u128, false>),
        p!("mmapmx_i8_f32", "mmapmx", 4, 150, 1, mmapmx::MxCase, mmapmx::gen_case(), mmapmx::run_file::<i8, f32, false>),
        p!("mmapmx_u16_arr33", "mmapmx", 4, 150, 1, mmapmx::MxCase, mmapmx::gen_case(), mmapmx::run_file::<u16, [u8; 33], false>),
        p!("mmapmx_i128_i16", "mmapmx", 4, 150, 1, mmapmx::MxCase, mmapmx::gen_case(), mmapmx::run_file::<i128, i16, true>),
        p!("mmapmx_userid_f64", "mmapmx", 4, 150, 1, mmapmx::MxCase, mmapmx::gen_case(), mmapmx::run_file::<mmapmx::UserId, f64, true>),
        p!("mmapmx_uk5w2_u8", "mmapmx", 4, 150, 1, mmapmx::MxCase, mmapmx::gen_case(), mmapmx::run_file::<mmapmx::Uk5W2, u8, true>),
        p!("mmapmx_uk12w2_u128", "mmapmx", 4, 150, 1, mmapmx::MxCase, mmapmx::gen_case(), mmapmx::run_file::<mmapmx::Uk12W2, u128, true>),
        p!("mmapmx_uk24w4_arr5", "mmapmx", 4, 150, 1, mmapmx::MxCase, mmapmx::gen_case(), mmapmx::run_file::<mmapmx::Uk24W4, [u8; 5], true>),
        p!("mmapmx_uk70w8_i16", "mmapmx", 4, 150, 1, mmapmx::MxCase, mmapmx::gen_case(), mmapmx::run_file::<mmapmx::Uk70W8, i16, true>),
        p!("mmapmx_uk9w4_zst", "mmapmx", 4, 150, 1, mmapmx::MxCase, mmapmx::gen_case(), mmapmx::run_file::<mmapmx::Uk9W4, [u8; 0], true>),
        p!("mmapmx_arr20_text", "mmapmx", 4, 150, 1, mmapmx::MxCase, mmapmx::gen_case(), mmapmx::run::<[u8; 20], String, true>),
        p!("mmapmx_uk12w2_bytes", "mmapmx", 4, 150, 1, mmapmx::MxCase, mmapmx::gen_case(), mmapmx::run::<mmapmx::Uk12W2, Vec<u8>, true>),
        p!("mmapmx_i8_bytes", "mmapmx", 4, 150, 1, mmapmx::MxCase, mmapmx::gen_case(), mmapmx::run::<i8, Vec<u8>, false>),
        p!("mmapmx_uk70w8_text", "mmapmx", 4, 150, 1, mmapmx::MxCase, mmapmx::gen_case(), mmapmx::run::<mmapmx::Uk70W8, String, true>),
        // bbtree-store, single-threaded with page refusals
        p!("store_default", "store", 6, 150, 1, store::Case, store::case(), store::run::<store::Defaults>),
        p!("store_small", "store", 6, 150, 1, store::Case, store::case(), store::run::<store::Small>),
        p!("store_big_heap", "store", 6, 150, 1, store::Case, store::case(), store::run::<store::BigHeap>),
        p!("store_big_leaves", "store", 6, 150, 1, store::Case, store::case(), store::run::<store::BigLeaves>),
        // bbtree-store shared by threads
        p!("storethreads_default", "storethreads", 6, 100, 20, storethreads::Case, storethreads::case(), storethreads::run::<store::Defaults>),
        p!("storethreads_small", "storethreads", 6, 100, 20, storethreads::Case, storethreads::case(), storethreads::run::<storethreads::SmallInternals>),
        // bbtree-store threads checked while they run: own keys against each thread's model, thread-local page refusals, hot keys
        p!("storelin_default", "storelin", 8, 50, 20, storelin::Case, storelin::case(), storelin::run::<store::Defaults>),
        p!("storelin_small", "storelin", 8, 50, 20, storelin::Case, storelin::case(), storelin::run::<store::Small>),
        p!("storelin_big_heap", "storelin", 6, 50, 20, storelin::Case, storelin::case(), storelin::run::<store::BigHeap>),
        p!("storelin_big_leaves", "storelin", 6, 50, 20, storelin::Case, storelin::case(), storelin::run::<store::BigLeaves>),
        // bbtree::BBTreeMap under page refusals
        p!("syncmap_alloc_u64", "syncmap", 8, 200, 1, syncalloc::Case, syncalloc::case(), syncalloc::run::<u64>),
        p!("syncmap_alloc_string", "syncmap", 8, 200, 1, syncalloc::Case, syncalloc::case(), syncalloc::run::<String>),
        // panics in closures and in keys' and values' Ord/Clone/Drop: latches released, half changes kept, teardown order and counts
        p!("syncpanic_pk_inline", "syncpanic", 6, 1000, 20, syncpanic::Case, syncpanic::case(), syncpanic::run::<syncpanic::PK, bbtree::WithInlineKey, false>),
        p!("syncpanic_pk_inline_narrow", "syncpanic", 6, 1000, 20, syncpanic::Case, syncpanic::case(), syncpanic::run::<syncpanic::PK, bbtree::Configured<bbtree::WithInlineKey, syncthreads::Narrow>, false>),
        p!("syncpanic_u64", "syncpanic", 4, 1000, 20, syncpanic::Case, syncpanic::case(), syncpanic::run::<u64, Auto, false>),
        p!("syncpanic_string", "syncpanic", 4, 1000, 20, syncpanic::Case, syncpanic::case(), syncpanic::run::<String, Auto, false>),
        p!("syncpanic_string_inline_narrow", "syncpanic", 3, 1000, 20, syncpanic::Case, syncpanic::case(), syncpanic::run::<String, bbtree::Configured<bbtree::WithInlineKey, syncthreads::Narrow>, false>),
        // the same with get_or_insert's closure panicking on new entries (it runs inside a split)
        p!("syncvacant_u64", "syncvacant", 2, 100, 20, syncpanic::Case, syncpanic::case(), syncpanic::run::<u64, Auto, true>),
        p!("syncvacant_pk_inline_narrow", "syncvacant", 2, 100, 20, syncpanic::Case, syncpanic::case(), syncpanic::run::<syncpanic::PK, bbtree::Configured<bbtree::WithInlineKey, syncthreads::Narrow>, true>),
        p!("syncpanic_store", "syncpanic", 5, 1000, 20, syncpanic::SCase, syncpanic::scase(), syncpanic::run_store),
        // unsync map at large sizes, removal-heavy
        p!("bulkmap_u64", "bulkmap", 3, 20, 1, bulk::Case, bulk::case(), bulk::run::<u64, Auto>),
        p!("bulkmap_i128", "bulkmap", 3, 20, 1, bulk::Case, bulk::case(), bulk::run::<i128, Auto>),
        p!("bulkmap_string", "bulkmap", 3, 20, 1, bulk::Case, bulk::case(), bulk::run::<String, Auto>),
        p!("bulkmap_bytes", "bulkmap", 3, 20, 1, bulk::Case, bulk::case(), bulk::run::<Vec<u8>, Auto>),
        p!("bulkmap_arr16", "bulkmap", 3, 20, 1, bulk::Case, bulk::case(), bulk::run::<[u8; 16], Auto>),
        p!("bulkmap_u64_narrow", "bulkmap", 3, 20, 1, bulk::Case, bulk::case(), bulk::run::<u64, bbtree::Configured<Auto, bulk::Narrow>>),
        p!("bulkmap_string_narrow", "bulkmap", 3, 20, 1, bulk::Case, bulk::case(), bulk::run::<String, bbtree::Configured<Auto, bulk::Narrow>>),
        p!("bulkmap_string_inline_narrow", "bulkmap", 3, 20, 1, bulk::Case, bulk::case(), bulk::run::<String, bbtree::Configured<bbtree::WithInlineKey, bulk::Narrow>>),
        p!("bulkmap_bytes_wide", "bulkmap", 3, 20, 1, bulk::Case, bulk::case(), bulk::run::<Vec<u8>, bbtree::Configured<Auto, bulk::Wide>>),
        p!("bulkmap_i128_wide", "bulkmap", 3, 20, 1, bulk::Case, bulk::case(), bulk::run::<i128, bbtree::Configured<Auto, bulk::Wide>>),
        // unsync map and set: a stateful model over the entry API, mutable and
        // consuming walks, clone (with panics), set algebra and relations
        p!("umodel_u64", "umodel", 5, 50, 1, umodel::Case, umodel::case(), umodel::run::<u64, umodel::Val<0>, Auto>),
        p!("umodel_i128_pad3", "umodel", 5, 50, 1, umodel::Case, umodel::case(), umodel::run::<i128, umodel::Val<3>, Auto>),
        p!("umodel_arr20", "umodel", 5, 50, 1, umodel::Case, umodel::case(), umodel::run::<[u8; 20], umodel::Val<0>, Auto>),
        p!("umodel_string", "umodel", 5, 50, 1, umodel::Case, umodel::case(), umodel::run::<String, umodel::Val<0>, Auto>),
        p!("umodel_bytes_inline", "umodel", 5, 50, 1, umodel::Case, umodel::case(), umodel::run::<Vec<u8>, umodel::Val<0>, bbtree::WithInlineKey>),
        p!("umodel_boxbytes_inline_narrow", "umodel", 5, 50, 1, umodel::Case, umodel::case(), umodel::run::<Box<[u8]>, umodel::Val<3>, bbtree::Configured<bbtree::WithInlineKey, bulk::Narrow>>),
        p!("umodel_string_inline_pad40", "umodel", 5, 50, 1, umodel::Case, umodel::case(), umodel::run::<String, umodel::Val<40>, bbtree::WithInlineKey>),
        p!("umodel_u64_narrow_pad40", "umodel", 5, 50, 1, umodel::Case, umodel::case(), umodel::run::<u64, umodel::Val<40>, bbtree::Configured<Auto, bulk::Narrow>>),
        p!("umodel_i128_wide", "umodel", 5, 50, 1, umodel::Case, umodel::case(), umodel::run::<i128, umodel::Val<0>, bbtree::Configured<Auto, bulk::Wide>>),
        // keys with a tag Ord ignores: which of two equal keys the map keeps (insert, entry, set replace, append)
        p!("umodel_tagstr_inline", "umodel", 5, 50, 1, umodel::Case, umodel::case(), umodel::run::<umodel::Tagged, umodel::Val<0>, bbtree::WithInlineKey>),
        p!("umodel_tagstr_inline_narrow_pad3", "umodel", 5, 50, 1, umodel::Case, umodel::case(), umodel::run::<umodel::Tagged, umodel::Val<3>, bbtree::Configured<bbtree::WithInlineKey, bulk::Narrow>>),
        p!("umodel_tagstr_plain", "umodel", 3, 50, 1, umodel::Case, umodel::case(), umodel::run::<umodel::Tagged, umodel::Val<0>, Auto>),
        p!("umodel_tagu64_inline_narrow", "umodel", 5, 50, 1, umodel::Case, umodel::case(), umodel::run::<umodel::TagU64, umodel::Val<0>, bbtree::Configured<bbtree::WithInlineKey, bulk::Narrow>>),
        p!("umodel_sync_tagstr_inline", "umodel", 4, 50, 1, umodel::Case, umodel::case(), umodel::run_sync::<umodel::Tagged, umodel::Val<0>, bbtree::WithInlineKey>),
        p!("umodel_sync_tagu64_inline_pad3", "umodel", 4, 50, 1, umodel::Case, umodel::case(), umodel::run_sync::<umodel::TagU64, umodel::Val<3>, bbtree::WithInlineKey>),
        p!("umodel_sync_u64_pad3", "umodel", 4, 50, 1, umodel::Case, umodel::case(), umodel::run_sync::<u64, umodel::Val<3>, Auto>),
        p!("umodel_sync_string", "umodel", 4, 50, 1, umodel::Case, umodel::case(), umodel::run_sync::<String, umodel::Val<0>, Auto>),
        p!("umodel_sync_bytes_inline_narrow", "umodel", 4, 50, 1, umodel::Case, umodel::case(), umodel::run_sync::<Vec<u8>, umodel::Val<0>, bbtree::Configured<bbtree::WithInlineKey, bulk::Narrow>>),
        // typemx: the umodel model over key and value types no other
        // property uses (node layouts of tiny, huge and over-aligned slots).
        p!("typemx_u8_a64", "typemx", 4, 50, 1, umodel::Case, umodel::case(), umodel::tm::<u8, umodel::Val<1, umodel::A64>, Auto>),
        p!("typemx_i16_big", "typemx", 4, 50, 1, umodel::Case, umodel::case(), umodel::tm::<i16, umodel::Val<300>, Auto>),
        p!("typemx_u32_a16", "typemx", 4, 50, 1, umodel::Case, umodel::case(), umodel::tm::<u32, umodel::Val<0, umodel::A16>, Auto>),
        p!("typemx_char", "typemx", 4, 50, 1, umodel::Case, umodel::case(), umodel::tm::<char, umodel::Val<0>, Auto>),
        p!("typemx_bool_a128", "typemx", 4, 50, 1, umodel::Case, umodel::case(), umodel::tm::<bool, umodel::Val<0, umodel::A128>, Auto>),
        p!("typemx_unit", "typemx", 4, 50, 1, umodel::Case, umodel::case(), umodel::tm::<(), umodel::Val<2>, Auto>),
        p!("typemx_tuple_narrow", "typemx", 4, 50, 1, umodel::Case, umodel::case(), umodel::tm::<(u8, u16), umodel::Val<0>, bbtree::Configured<Auto, bulk::Narrow>>),
        p!("typemx_i16_wide", "typemx", 4, 50, 1, umodel::Case, umodel::case(), umodel::tm::<i16, umodel::Val<0>, bbtree::Configured<Auto, bulk::Wide>>),
        p!("typemx_optu64_pad130", "typemx", 4, 50, 1, umodel::Case, umodel::case(), umodel::tm::<Option<u64>, umodel::Val<130>, Auto>),
        p!("typemx_k64", "typemx", 4, 50, 1, umodel::Case, umodel::case(), umodel::tm::<umodel::K64, umodel::Val<0>, Auto>),
        p!("typemx_k64_a128_wide", "typemx", 4, 50, 1, umodel::Case, umodel::case(), umodel::tm::<umodel::K64, umodel::Val<5, umodel::A128>, bbtree::Configured<Auto, bulk::Wide>>),
        p!("typemx_arr3_inline", "typemx", 4, 50, 1, umodel::Case, umodel::case(), umodel::tm::<[u8; 3], umodel::Val<0>, bbtree::WithInlineKey>),
        p!("typemx_arr3_a64", "typemx", 4, 50, 1, umodel::Case, umodel::case(), umodel::tm::<[u8; 3], umodel::Val<0, umodel::A64>, Auto>),
        p!("typemx_arr300_inline", "typemx", 4, 50, 1, umodel::Case, umodel::case(), umodel::tm::<[u8; 300], umodel::Val<0>, bbtree::WithInlineKey>),
        p!("typemx_arr64_big", "typemx", 4, 50, 1, umodel::Case, umodel::case(), umodel::tm::<[u8; 64], umodel::Val<300>, Auto>),
        p!("typemx_u64_big", "typemx", 4, 50, 1, umodel::Case, umodel::case(), umodel::tm::<u64, umodel::Val<300>, Auto>),
        p!("typemx_u64_a128_narrow", "typemx", 4, 50, 1, umodel::Case, umodel::case(), umodel::tm::<u64, umodel::Val<0, umodel::A128>, bbtree::Configured<Auto, bulk::Narrow>>),
        p!("typemx_arr16_a64", "typemx", 4, 50, 1, umodel::Case, umodel::case(), umodel::tm::<[u8; 16], umodel::Val<1, umodel::A64>, Auto>),
        p!("typemx_sync_u64_big", "typemx", 4, 50, 1, umodel::Case, umodel::case(), umodel::tm_sync::<u64, umodel::Val<300>, Auto>),
        p!("typemx_sync_arr3_inline_a64", "typemx", 4, 50, 1, umodel::Case, umodel::case(), umodel::tm_sync::<[u8; 3], umodel::Val<1, umodel::A64>, bbtree::WithInlineKey>),
        p!("typemx_sync_arr20_a128", "typemx", 4, 50, 1, umodel::Case, umodel::case(), umodel::tm_sync::<[u8; 20], umodel::Val<0, umodel::A128>, Auto>),
        p!("typemx_sync_arr300_inline", "typemx", 4, 50, 1, umodel::Case, umodel::case(), umodel::tm_sync::<[u8; 300], umodel::Val<0>, bbtree::WithInlineKey>),
        p!("typemx_sync_arr3_inline_big", "typemx", 4, 50, 1, umodel::Case, umodel::case(), umodel::tm_sync::<[u8; 3], umodel::Val<300>, bbtree::WithInlineKey>),
        p!("typemx_sync_string_a64", "typemx", 4, 50, 1, umodel::Case, umodel::case(), umodel::tm_sync::<String, umodel::Val<0, umodel::A64>, Auto>),
        // bbtree::BBTreeMap shared by threads, large trees
        p!("syncthreads_u64", "syncthreads", 4, 8, 20, syncthreads::Case, syncthreads::case(), syncthreads::run::<u64, Auto, false>),
        p!("syncthreads_string", "syncthreads", 4, 8, 20, syncthreads::Case, syncthreads::case(), syncthreads::run::<String, Auto, false>),
        p!("syncthreads_u64_narrow", "syncthreads", 4, 8, 20, syncthreads::Case, syncthreads::case(), syncthreads::run::<u64, bbtree::Configured<Auto, syncthreads::Narrow>, false>),
        // the same with keys outside the shared prefix: threads shrink it mid-run
        p!("syncthreads_u64_shrink", "syncthreads", 4, 8, 20, syncthreads::Case, syncthreads::case(), syncthreads::run::<u64, Auto, true>),
        p!("syncthreads_string_shrink", "syncthreads", 4, 8, 20, syncthreads::Case, syncthreads::case(), syncthreads::run::<String, Auto, true>),
        p!("syncthreads_string_shrink_narrow", "syncthreads", 4, 8, 20, syncthreads::Case, syncthreads::case(), syncthreads::run::<String, bbtree::Configured<Auto, syncthreads::Narrow>, true>),
        p!("syncthreads_string_inline_shrink", "syncthreads", 4, 8, 20, syncthreads::Case, syncthreads::case(), syncthreads::run::<String, bbtree::WithInlineKey, true>),
        // bbtree_store::wal: lost outcomes, rival writers, damage, against the documented layout
        p!("wal_max1", "wal", 8, 200, 1, wal::Case, wal::case(), wal::run::<1>),
        p!("wal_max2", "wal", 8, 200, 1, wal::Case, wal::case(), wal::run::<2>),
        p!("wal_max3", "wal", 8, 200, 1, wal::Case, wal::case(), wal::run::<3>),
        p!("wal_max7", "wal", 8, 200, 1, wal::Case, wal::case(), wal::run::<7>),
        p!("wal_max10000", "wal", 8, 200, 1, wal::Case, wal::case(), wal::run::<10_000>),
        p!("syncthreads_string_narrow", "syncthreads", 4, 8, 20, syncthreads::Case, syncthreads::case(), syncthreads::run::<String, bbtree::Configured<Auto, syncthreads::Narrow>, false>),
        // bbtree::BBTreeMap with a leaf annex: update_or_insert_with_leaf / remove_with_leaf, a probe annex through splits, merges, teardown
        p!("annex_u64", "annex", 5, 60, 1, annex::Case, annex::case(), annex::run::<u64, Auto>),
        p!("annex_string", "annex", 5, 60, 1, annex::Case, annex::case(), annex::run::<String, Auto>),
        p!("annex_string_narrow", "annex", 5, 60, 1, annex::Case, annex::case(), annex::run::<String, bbtree::Configured<Auto, bulk::Narrow>>),
        p!("annex_bytes_inline", "annex", 5, 60, 1, annex::Case, annex::case(), annex::run::<Vec<u8>, bbtree::WithInlineKey>),
        p!("annex_threads_u64", "annex", 4, 10, 20, annex::TCase, annex::tcase(), annex::run_threads::<u64, Auto>),
        p!("annex_threads_string_narrow", "annex", 4, 10, 20, annex::TCase, annex::tcase(), annex::run_threads::<String, bbtree::Configured<Auto, bulk::Narrow>>),
        p!("annex_threads_bytes_inline", "annex", 4, 10, 20, annex::TCase, annex::tcase(), annex::run_threads::<Vec<u8>, bbtree::WithInlineKey>),
        // bbtree::BBTreeMap threads checked while they run: own keys against each thread's model, hot counters conserved and monotone
        p!("synclin_u64_w4", "synclin", 4, 8, 20, synclin::Case, synclin::case(), synclin::run::<u64, Auto, 4, false>),
        p!("synclin_u64_narrow_w3", "synclin", 4, 8, 20, synclin::Case, synclin::case(), synclin::run::<u64, bbtree::Configured<Auto, syncthreads::Narrow>, 3, false>),
        p!("synclin_u64_shrink_w8", "synclin", 3, 8, 20, synclin::Case, synclin::case(), synclin::run::<u64, Auto, 8, true>),
        p!("synclin_string_shrink_w8", "synclin", 4, 8, 20, synclin::Case, synclin::case(), synclin::run::<String, Auto, 8, true>),
        p!("synclin_string_narrow_w3", "synclin", 3, 8, 20, synclin::Case, synclin::case(), synclin::run::<String, bbtree::Configured<Auto, syncthreads::Narrow>, 3, false>),
        p!("synclin_string_inline_shrink_w4", "synclin", 3, 8, 20, synclin::Case, synclin::case(), synclin::run::<String, bbtree::WithInlineKey, 4, true>),
        // user InlineKey impls under WithInlineKey: every width, hash tags, cmp_from, cmp_prefix; call premises checked
        p!("ukey_umodel_w2", "ukey", 5, 50, 1, umodel::Case, umodel::case(), ukey::umodel::<2, 0>),
        p!("ukey_umodel_w4_tag", "ukey", 5, 50, 1, umodel::Case, umodel::case(), ukey::umodel::<4, 1>),
        p!("ukey_umodel_w8_cmp", "ukey", 5, 50, 1, umodel::Case, umodel::case(), ukey::umodel::<8, 2>),
        p!("ukey_umodel_w8_all", "ukey", 5, 50, 1, umodel::Case, umodel::case(), ukey::umodel::<8, 7>),
        p!("ukey_umodel_w2_tag_cmp", "ukey", 5, 50, 1, umodel::Case, umodel::case(), ukey::umodel::<2, 3>),
        p!("ukey_umodel_w4_cmp_prefix", "ukey", 5, 50, 1, umodel::Case, umodel::case(), ukey::umodel::<4, 6>),
        p!("ukey_umodel_narrow_w8_tag_cmp", "ukey", 5, 50, 1, umodel::Case, umodel::case(), ukey::umodel_narrow::<8, 3>),
        p!("ukey_umodel_narrow_w2_tag_prefix", "ukey", 5, 50, 1, umodel::Case, umodel::case(), ukey::umodel_narrow::<2, 5>),
        p!("ukey_umodel_sync_w4_tag_cmp", "ukey", 5, 50, 1, umodel::Case, umodel::case(), ukey::umodel_sync::<4, 3>),
        p!("ukey_umodel_sync_w8", "ukey", 5, 50, 1, umodel::Case, umodel::case(), ukey::umodel_sync::<8, 0>),
        p!("ukey_umodel_sync_w2_all", "ukey", 5, 50, 1, umodel::Case, umodel::case(), ukey::umodel_sync::<2, 7>),
        p!("ukey_annex_w2_tag", "ukey", 5, 60, 1, annex::Case, annex::case(), ukey::annex::<2, 1>),
        p!("ukey_annex_w8_cmp_prefix", "ukey", 5, 60, 1, annex::Case, annex::case(), ukey::annex::<8, 6>),
        p!("ukey_annex_threads_w4_tag_cmp", "ukey", 5, 10, 20, annex::TCase, annex::tcase(), ukey::annex_threads::<4, 3>),
        p!("ukey_synclin_w4_cmp", "ukey", 5, 8, 20, synclin::Case, synclin::case(), ukey::synclin::<4, 2>),
        p!("ukey_synclin_w8_tag_prefix", "ukey", 5, 8, 20, synclin::Case, synclin::case(), ukey::synclin::<8, 5>),
        p!("ukey_bulk_w2_cmp", "ukey", 5, 20, 1, bulk::Case, bulk::case(), ukey::bulk::<2, 2>),
        p!("ukey_bulk_w8_tag_prefix", "ukey", 5, 20, 1, bulk::Case, bulk::case(), ukey::bulk::<8, 5>),
        // shlin: threads writing and removing the same keys, each key's history checked for linearizability
        p!("shlin_u64_w3", "shlin", 4, 60, 20, shlin::Case, shlin::case(), shlin::run::<u64, Auto, 3, false>),
        p!("shlin_u64_narrow_w2", "shlin", 4, 60, 20, shlin::Case, shlin::case(), shlin::run::<u64, bbtree::Configured<Auto, syncthreads::Narrow>, 2, false>),
        p!("shlin_u64_shrink_w4", "shlin", 3, 60, 20, shlin::Case, shlin::case(), shlin::run::<u64, Auto, 4, true>),
        p!("shlin_string_shrink_w3", "shlin", 4, 60, 20, shlin::Case, shlin::case(), shlin::run::<String, Auto, 3, true>),
        p!("shlin_string_inline_narrow_w2", "shlin", 3, 60, 20, shlin::Case, shlin::case(), shlin::run::<String, bbtree::Configured<bbtree::WithInlineKey, syncthreads::Narrow>, 2, false>),
        p!("shlin_u64_w130", "shlin", 2, 60, 20, shlin::Case, shlin::case(), shlin::run::<u64, Auto, 130, false>),
        // shlinr: the shlin threads on an allocator that refuses pages to each thread at its own rate
        p!("shlinr_u64_w3", "shlinr", 4, 60, 20, shlin::RCase, shlin::rcase(), shlin::run_refuse::<u64, Auto, 3, false>),
        p!("shlinr_u64_narrow_w2", "shlinr", 4, 60, 20, shlin::RCase, shlin::rcase(), shlin::run_refuse::<u64, bbtree::Configured<Auto, syncthreads::Narrow>, 2, false>),
        p!("shlinr_string_shrink_w3", "shlinr", 4, 60, 20, shlin::RCase, shlin::rcase(), shlin::run_refuse::<String, Auto, 3, true>),
        p!("shlinr_string_inline_narrow_w2", "shlinr", 3, 60, 20, shlin::RCase, shlin::rcase(), shlin::run_refuse::<String, bbtree::Configured<bbtree::WithInlineKey, syncthreads::Narrow>, 2, false>),
        p!("shlin_store_default_short", "shlin", 3, 60, 20, shlin::Case, shlin::case(), shlin::run_store::<store::Defaults, 0>),
        p!("shlin_store_small_prefix", "shlin", 3, 60, 20, shlin::Case, shlin::case(), shlin::run_store::<store::Small, 1>),
        p!("shlin_store_big_heap_long", "shlin", 2, 60, 20, shlin::Case, shlin::case(), shlin::run_store::<store::BigHeap, 2>),
        p!("shlin_store_big_leaves_shrink", "shlin", 2, 60, 20, shlin::Case, shlin::case(), shlin::run_store::<store::BigLeaves, 3>),
        // sametree: insert_many / extend / clone give the same nodes as one insert at a time (design/12, tree/clone.rs)
        p!("sametree_u64", "sametree", 3, 30, 1, sametree::Case, sametree::case(), sametree::run::<u64, umodel::Val<0>, Auto>),
        p!("sametree_u64_narrow_pad3", "sametree", 3, 30, 1, sametree::Case, sametree::case(), sametree::run::<u64, umodel::Val<3>, bbtree::Configured<Auto, bulk::Narrow>>),
        p!("sametree_i128_wide", "sametree", 3, 30, 1, sametree::Case, sametree::case(), sametree::run::<i128, umodel::Val<0>, bbtree::Configured<Auto, bulk::Wide>>),
        p!("sametree_arr20", "sametree", 3, 30, 1, sametree::Case, sametree::case(), sametree::run::<[u8; 20], umodel::Val<0>, Auto>),
        p!("sametree_string", "sametree", 3, 30, 1, sametree::Case, sametree::case(), sametree::run::<String, umodel::Val<0>, Auto>),
        p!("sametree_string_inline_pad40", "sametree", 3, 30, 1, sametree::Case, sametree::case(), sametree::run::<String, umodel::Val<40>, bbtree::WithInlineKey>),
        p!("sametree_bytes_inline_narrow", "sametree", 3, 30, 1, sametree::Case, sametree::case(), sametree::run::<Vec<u8>, umodel::Val<0>, bbtree::Configured<bbtree::WithInlineKey, bulk::Narrow>>),
        p!("sametree_tagstr_inline", "sametree", 3, 30, 1, sametree::Case, sametree::case(), sametree::run::<umodel::Tagged, umodel::Val<0>, bbtree::WithInlineKey>),
        p!("sametree_tagu64_inline_narrow", "sametree", 3, 30, 1, sametree::Case, sametree::case(), sametree::run::<umodel::TagU64, umodel::Val<0>, bbtree::Configured<bbtree::WithInlineKey, bulk::Narrow>>),
        p!("sametree_u8_a64", "sametree", 3, 30, 1, sametree::Case, sametree::case(), sametree::run::<u8, umodel::Val<1, umodel::A64>, Auto>),
        p!("sametree_i16_wide", "sametree", 3, 30, 1, sametree::Case, sametree::case(), sametree::run::<i16, umodel::Val<0>, bbtree::Configured<Auto, bulk::Wide>>),
        p!("sametree_unit", "sametree", 3, 30, 1, sametree::Case, sametree::case(), sametree::run::<(), umodel::Val<2>, Auto>),
        p!("sametree_arr3_inline", "sametree", 3, 30, 1, sametree::Case, sametree::case(), sametree::run::<[u8; 3], umodel::Val<0>, bbtree::WithInlineKey>),
        p!("sametree_arr300_inline", "sametree", 3, 30, 1, sametree::Case, sametree::case(), sametree::run::<[u8; 300], umodel::Val<0>, bbtree::WithInlineKey>),
        p!("sametree_k64_a128_wide", "sametree", 3, 30, 1, sametree::Case, sametree::case(), sametree::run::<umodel::K64, umodel::Val<5, umodel::A128>, bbtree::Configured<Auto, bulk::Wide>>),
        p!("sametree_ukey_w4_tag", "sametree", 3, 30, 1, sametree::Case, sametree::case(), ukey::sametree::<4, 1>),
        p!("sametree_ukey_w8_all", "sametree", 3, 30, 1, sametree::Case, sametree::case(), ukey::sametree::<8, 7>),
        p!("sametree_ukey_narrow_w8_tag_cmp", "sametree", 3, 30, 1, sametree::Case, sametree::case(), ukey::sametree_narrow::<8, 3>),
        p!("sametree_ukey_narrow_w2_tag_prefix", "sametree", 3, 30, 1, sametree::Case, sametree::case(), ukey::sametree_narrow::<2, 5>),
        // upanic: panics in an unsync map's user code (Ord, InlineKey, Drop, closures, extend's iterator) against the exact states the docs promise after one
        p!("upanic_int_auto", "upanic", 9, 40, 1, upanic::Case, upanic::case(), upanic::run::<upanic::IntK, Auto>),
        p!("upanic_int_narrow", "upanic", 8, 40, 1, upanic::Case, upanic::case(), upanic::run::<upanic::IntK, bbtree::Configured<Auto, bulk::Narrow>>),
        p!("upanic_str_inline", "upanic", 9, 40, 1, upanic::Case, upanic::case(), upanic::run::<upanic::StrK, bbtree::WithInlineKey>),
        p!("upanic_str_inline_narrow", "upanic", 8, 40, 1, upanic::Case, upanic::case(), upanic::run::<upanic::StrK, bbtree::Configured<bbtree::WithInlineKey, bulk::Narrow>>),
        p!("upanic_ik_inline", "upanic", 9, 40, 1, upanic::Case, upanic::case(), upanic::run::<upanic::IkK, bbtree::WithInlineKey>),
        p!("upanic_ik_inline_narrow", "upanic", 8, 40, 1, upanic::Case, upanic::case(), upanic::run::<upanic::IkK, bbtree::Configured<bbtree::WithInlineKey, bulk::Narrow>>),
        p!("upanic_ik_inline_wide", "upanic", 6, 40, 1, upanic::Case, upanic::case(), upanic::run::<upanic::IkK, bbtree::Configured<bbtree::WithInlineKey, bulk::Wide>>),
        // sametree with keys whose Ord panics at an armed call (D48: only soundness after)
        p!("sametree_bomb_u64_plain", "sametree", 3, 30, 1, sametree::Case, sametree::case(), sametree::run::<sametree::BombKey, umodel::Val<0>, Auto>),
        p!("sametree_bomb_str_inline", "sametree", 3, 30, 1, sametree::Case, sametree::case(), sametree::run::<sametree::BombStr, umodel::Val<3>, bbtree::WithInlineKey>),
        p!("sametree_bomb_str_inline_narrow", "sametree", 3, 30, 1, sametree::Case, sametree::case(), sametree::run::<sametree::BombStr, umodel::Val<0>, bbtree::Configured<bbtree::WithInlineKey, bulk::Narrow>>),
        // pagemx: the umodel/sametree/bulkmap/filemap/store/synclin models under node configs of 16 KiB-256 KiB leaves and 16-64 KiB internal nodes (capacities at the 1023-key cap)
        p!("pagemx_umodel_u64_l16k", "pagemx", 3, 50, 1, umodel::Case, umodel::case(), pagemx::umodel::<u64, umodel::Val<0>, pagemx::A<pagemx::L16K>>),
        p!("pagemx_umodel_i16_huge", "pagemx", 3, 50, 1, umodel::Case, umodel::case(), pagemx::umodel::<i16, umodel::Val<0>, pagemx::A<pagemx::Huge>>),
        p!("pagemx_umodel_u32_i64k", "pagemx", 3, 50, 1, umodel::Case, umodel::case(), pagemx::umodel::<u32, umodel::Val<0>, pagemx::A<pagemx::I64K>>),
        p!("pagemx_umodel_string_l64ktiny", "pagemx", 3, 50, 1, umodel::Case, umodel::case(), pagemx::umodel::<String, umodel::Val<0>, pagemx::A<pagemx::L64KTiny>>),
        p!("pagemx_umodel_bytes_inline_huge", "pagemx", 3, 50, 1, umodel::Case, umodel::case(), pagemx::umodel::<Vec<u8>, umodel::Val<0>, pagemx::I<pagemx::Huge>>),
        p!("pagemx_umodel_tagstr_inline_mixed", "pagemx", 3, 50, 1, umodel::Case, umodel::case(), pagemx::umodel::<umodel::Tagged, umodel::Val<0>, pagemx::I<pagemx::Mixed>>),
        p!("pagemx_umodel_arr20_mixed_pad40", "pagemx", 3, 50, 1, umodel::Case, umodel::case(), pagemx::umodel::<[u8; 20], umodel::Val<40>, pagemx::A<pagemx::Mixed>>),
        p!("pagemx_umodel_u8_l64ktiny", "pagemx", 2, 50, 1, umodel::Case, umodel::case(), pagemx::umodel::<u8, umodel::Val<0>, pagemx::A<pagemx::L64KTiny>>),
        p!("pagemx_umodel_i128_i64k_pad3", "pagemx", 3, 50, 1, umodel::Case, umodel::case(), pagemx::umodel::<i128, umodel::Val<3>, pagemx::A<pagemx::I64K>>),
        p!("pagemx_umodel_sync_u64_huge", "pagemx", 3, 50, 1, umodel::Case, umodel::case(), pagemx::umodel_sync::<u64, umodel::Val<0>, pagemx::A<pagemx::Huge>>),
        p!("pagemx_umodel_sync_string_inline_l64ktiny", "pagemx", 3, 50, 1, umodel::Case, umodel::case(), pagemx::umodel_sync::<String, umodel::Val<0>, pagemx::I<pagemx::L64KTiny>>),
        p!("pagemx_umodel_sync_arr32_i64k", "pagemx", 3, 50, 1, umodel::Case, umodel::case(), pagemx::umodel_sync::<[u8; 32], umodel::Val<0>, pagemx::A<pagemx::I64K>>),
        p!("pagemx_sametree_u64_huge", "pagemx", 3, 30, 1, sametree::Case, sametree::case(), pagemx::sametree::<u64, umodel::Val<0>, pagemx::A<pagemx::Huge>>),
        p!("pagemx_sametree_string_inline_mixed", "pagemx", 3, 30, 1, sametree::Case, sametree::case(), pagemx::sametree::<String, umodel::Val<0>, pagemx::I<pagemx::Mixed>>),
        p!("pagemx_sametree_u32_l64ktiny", "pagemx", 3, 30, 1, sametree::Case, sametree::case(), pagemx::sametree::<u32, umodel::Val<0>, pagemx::A<pagemx::L64KTiny>>),
        p!("pagemx_sametree_i16_i64k", "pagemx", 3, 30, 1, sametree::Case, sametree::case(), pagemx::sametree::<i16, umodel::Val<0>, pagemx::A<pagemx::I64K>>),
        p!("pagemx_bulk_u64_l64ktiny", "pagemx", 3, 20, 1, bulk::Case, bulk::case(), pagemx::bulk::<u64, pagemx::A<pagemx::L64KTiny>>),
        p!("pagemx_bulk_string_huge", "pagemx", 3, 20, 1, bulk::Case, bulk::case(), pagemx::bulk::<String, pagemx::A<pagemx::Huge>>),
        p!("pagemx_bulk_u32_i64k", "pagemx", 3, 20, 1, bulk::Case, bulk::case(), pagemx::bulk::<u32, pagemx::A<pagemx::I64K>>),
        p!("pagemx_bulk_arr16_mixed", "pagemx", 3, 20, 1, bulk::Case, bulk::case(), pagemx::bulk::<[u8; 16], pagemx::A<pagemx::Mixed>>),
        p!("pagemx_bulk_bytes_inline_l16k", "pagemx", 3, 20, 1, bulk::Case, bulk::case(), pagemx::bulk::<Vec<u8>, pagemx::I<pagemx::L16K>>),
        p!("pagemx_filemap_u64_u8_huge", "pagemx", 2, 200, 1, FR, frg(), pagemx::filemap::<u64, u8, pagemx::A<pagemx::Huge>>),
        p!("pagemx_filemap_u16_u64_l64ktiny", "pagemx", 2, 200, 1, FR, frg(), pagemx::filemap::<u16, u64, pagemx::A<pagemx::L64KTiny>>),
        p!("pagemx_filemap_arr20_u32_i64k", "pagemx", 2, 200, 1, FR, frg(), pagemx::filemap::<[u8; 20], u32, pagemx::A<pagemx::I64K>>),
        p!("pagemx_filemap_u32_arr3_mixed", "pagemx", 2, 200, 1, FR, frg(), pagemx::filemap::<u32, [u8; 3], pagemx::A<pagemx::Mixed>>),
        p!("pagemx_store_huge", "pagemx", 3, 150, 1, store::Case, store::case(), pagemx::store::<pagemx::StoreHuge>),
        p!("pagemx_store_mixed", "pagemx", 3, 150, 1, store::Case, store::case(), pagemx::store::<pagemx::Mixed>),
        p!("pagemx_long_auto", "pagemx", 2, 30, 1, pagemx::LongCase, pagemx::long_case(), pagemx::long_unsync::<Auto>),
        p!("pagemx_long_inline_l64ktiny", "pagemx", 2, 30, 1, pagemx::LongCase, pagemx::long_case(), pagemx::long_unsync::<pagemx::I<pagemx::L64KTiny>>),
        p!("pagemx_long_auto_narrow", "pagemx", 2, 30, 1, pagemx::LongCase, pagemx::long_case(), pagemx::long_unsync::<bbtree::Configured<Auto, bulk::Narrow>>),
        p!("pagemx_long_sync_auto", "pagemx", 2, 30, 1, pagemx::LongCase, pagemx::long_case(), pagemx::long_sync::<Auto>),
        p!("pagemx_long_sync_inline_narrow", "pagemx", 2, 30, 1, pagemx::LongCase, pagemx::long_case(), pagemx::long_sync::<bbtree::Configured<bbtree::WithInlineKey, syncthreads::Narrow>>),
        p!("pagemx_synclin_u64_huge_w4", "pagemx", 3, 8, 20, synclin::Case, synclin::case(), pagemx::synclin::<u64, pagemx::A<pagemx::Huge>, 4, false>),
        p!("pagemx_synclin_string_l64ktiny_shrink_w3", "pagemx", 3, 8, 20, synclin::Case, synclin::case(), pagemx::synclin::<String, pagemx::A<pagemx::L64KTiny>, 3, true>),
        p!("pagemx_synclin_u64_i64k_w2", "pagemx", 3, 8, 20, synclin::Case, synclin::case(), pagemx::synclin::<u64, pagemx::A<pagemx::I64K>, 2, false>),
        // values of 2.4 KB and 1 KB: leaves of 12 entries over several pages, copied whole by get_copied
        p!("synclin_u64_w300", "synclin", 3, 8, 20, synclin::Case, synclin::case(), synclin::run::<u64, Auto, 300, false>),
        p!("synclin_string_shrink_w130", "synclin", 3, 8, 20, synclin::Case, synclin::case(), synclin::run::<String, Auto, 130, true>),
        // synclinr: the synclin threads on an allocator that refuses pages to each thread at its own rate
        p!("synclinr_u64_w4", "synclinr", 4, 8, 20, synclin::RCase, synclin::rcase(), synclin::run_refuse::<u64, Auto, 4, false>),
        p!("synclinr_u64_narrow_w3", "synclinr", 4, 8, 20, synclin::RCase, synclin::rcase(), synclin::run_refuse::<u64, bbtree::Configured<Auto, syncthreads::Narrow>, 3, false>),
        p!("synclinr_string_shrink_w8", "synclinr", 4, 8, 20, synclin::RCase, synclin::rcase(), synclin::run_refuse::<String, Auto, 8, true>),
        p!("synclinr_string_narrow_w3", "synclinr", 3, 8, 20, synclin::RCase, synclin::rcase(), synclin::run_refuse::<String, bbtree::Configured<Auto, syncthreads::Narrow>, 3, false>),
        p!("synclinr_string_inline_shrink_w4", "synclinr", 3, 8, 20, synclin::RCase, synclin::rcase(), synclin::run_refuse::<String, bbtree::WithInlineKey, 4, true>),
        p!("synclinr_u64_w300", "synclinr", 3, 8, 20, synclin::RCase, synclin::rcase(), synclin::run_refuse::<u64, Auto, 300, false>),
        // zkey: the exact oracles over trie-preorder byte strings ({0, 1, 'a', 0x7f}: empty, prefix-of-each-other, trailing-zero keys; prefixes of the shared prefix)
        p!("zumodel_string", "zumodel", 4, 50, 1, umodel::Case, umodel::case(), zkey::umodel::<String, umodel::Val<0>, Auto>),
        p!("zumodel_bytes_inline", "zumodel", 4, 50, 1, umodel::Case, umodel::case(), zkey::umodel::<Vec<u8>, umodel::Val<0>, bbtree::WithInlineKey>),
        p!("zumodel_boxbytes_inline_narrow", "zumodel", 4, 50, 1, umodel::Case, umodel::case(), zkey::umodel::<Box<[u8]>, umodel::Val<3>, bbtree::Configured<bbtree::WithInlineKey, bulk::Narrow>>),
        p!("zumodel_string_inline_pad40", "zumodel", 3, 50, 1, umodel::Case, umodel::case(), zkey::umodel::<String, umodel::Val<40>, bbtree::WithInlineKey>),
        p!("zumodel_tagstr_inline_narrow", "zumodel", 4, 50, 1, umodel::Case, umodel::case(), zkey::umodel::<umodel::Tagged, umodel::Val<0>, bbtree::Configured<bbtree::WithInlineKey, bulk::Narrow>>),
        p!("zumodel_bytes_wide", "zumodel", 3, 50, 1, umodel::Case, umodel::case(), zkey::umodel::<Vec<u8>, umodel::Val<0>, bbtree::Configured<Auto, bulk::Wide>>),
        p!("zumodel_sync_string", "zumodel", 4, 50, 1, umodel::Case, umodel::case(), zkey::umodel_sync::<String, umodel::Val<0>, Auto>),
        p!("zumodel_sync_bytes_inline_narrow", "zumodel", 4, 50, 1, umodel::Case, umodel::case(), zkey::umodel_sync::<Vec<u8>, umodel::Val<0>, bbtree::Configured<bbtree::WithInlineKey, bulk::Narrow>>),
        p!("zumodel_sync_tagstr_inline", "zumodel", 3, 50, 1, umodel::Case, umodel::case(), zkey::umodel_sync::<umodel::Tagged, umodel::Val<0>, bbtree::WithInlineKey>),
        p!("zsametree_string", "zsametree", 3, 30, 1, sametree::Case, sametree::case(), zkey::sametree::<String, umodel::Val<0>, Auto>),
        p!("zsametree_string_inline_pad40", "zsametree", 3, 30, 1, sametree::Case, sametree::case(), zkey::sametree::<String, umodel::Val<40>, bbtree::WithInlineKey>),
        p!("zsametree_bytes_inline_narrow", "zsametree", 3, 30, 1, sametree::Case, sametree::case(), zkey::sametree::<Vec<u8>, umodel::Val<0>, bbtree::Configured<bbtree::WithInlineKey, bulk::Narrow>>),
        p!("zbulkmap_bytes", "zbulkmap", 3, 20, 1, bulk::Case, bulk::case(), zkey::bulk::<Vec<u8>, Auto>),
        p!("zbulkmap_string_inline_narrow", "zbulkmap", 3, 20, 1, bulk::Case, bulk::case(), zkey::bulk::<String, bbtree::Configured<bbtree::WithInlineKey, bulk::Narrow>>),
        p!("zbulkmap_string_narrow", "zbulkmap", 2, 20, 1, bulk::Case, bulk::case(), zkey::bulk::<String, bbtree::Configured<Auto, bulk::Narrow>>),
        p!("zsyncthreads_string_shrink", "zsyncthreads", 3, 8, 20, syncthreads::Case, syncthreads::case(), zkey::syncthreads::<String, Auto, true>),
        p!("zsyncthreads_string_inline_shrink", "zsyncthreads", 3, 8, 20, syncthreads::Case, syncthreads::case(), zkey::syncthreads::<String, bbtree::WithInlineKey, true>),
        p!("zsynclin_string_shrink_w8", "zsynclin", 3, 8, 20, synclin::Case, synclin::case(), zkey::synclin::<String, Auto, 8, true>),
        p!("zsynclin_string_inline_shrink_w4", "zsynclin", 3, 8, 20, synclin::Case, synclin::case(), zkey::synclin::<String, bbtree::WithInlineKey, 4, true>),
        p!("zsynclin_string_narrow_w3", "zsynclin", 3, 8, 20, synclin::Case, synclin::case(), zkey::synclin::<String, bbtree::Configured<Auto, syncthreads::Narrow>, 3, false>),
        p!("zshlin_string_shrink_w3", "zshlin", 4, 60, 20, shlin::Case, shlin::case(), zkey::shlin::<String, Auto, 3, true>),
        p!("zshlin_string_inline_narrow_shrink_w2", "zshlin", 4, 60, 20, shlin::Case, shlin::case(), zkey::shlin::<String, bbtree::Configured<bbtree::WithInlineKey, syncthreads::Narrow>, 2, true>),
        p!("zshlin_string_inline_w2", "zshlin", 3, 60, 20, shlin::Case, shlin::case(), zkey::shlin::<String, bbtree::WithInlineKey, 2, false>),
        p!("zshlinr_string_shrink_w3", "zshlinr", 3, 60, 20, shlin::RCase, shlin::rcase(), zkey::shlinr::<String, Auto, 3, true>),
        p!("zshlinr_string_inline_narrow_shrink_w2", "zshlinr", 3, 60, 20, shlin::RCase, shlin::rcase(), zkey::shlinr::<String, bbtree::Configured<bbtree::WithInlineKey, syncthreads::Narrow>, 2, true>),
        p!("zsynclinr_string_shrink_w8", "zsynclinr", 3, 8, 20, synclin::RCase, synclin::rcase(), zkey::synclinr::<String, Auto, 8, true>),
        p!("zsynclinr_string_inline_shrink_w4", "zsynclinr", 3, 8, 20, synclin::RCase, synclin::rcase(), zkey::synclinr::<String, bbtree::WithInlineKey, 4, true>),
    ]
}

fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn epoch_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis()
}

fn hang_s() -> u64 {
    std::env::var("BBZ_HANG_S").ok().and_then(|s| s.parse().ok()).unwrap_or(30)
}

/// Panics are caught and reported by the harness; keep them off stderr
/// (threads spawned inside a property panic outside ephyra's capture).
fn quiet_panics() {
    if std::env::var("BBZ_PANIC_LOG").as_deref() != Ok("1") {
        std::panic::set_hook(Box::new(|_| {}));
    }
}

// ---------- worker ----------

fn worker(seed: u64, deadline_ms: u128, inflight: &Path) -> ExitCode {
    quiet_panics();
    let mut props = registry();
    // BBZ_ONLY=<family or name prefix>: only those properties (for trying
    // one oracle out; never set in a search).
    if let Ok(only) = std::env::var("BBZ_ONLY") {
        props.retain(|p| p.family == only || p.name.starts_with(&only));
    }
    // the chaos build perturbs only threads' interleavings (chaos.rs)
    if chaos::ACTIVE {
        props.retain(|p| p.attempts > 1);
    }
    let total: u32 = props.iter().map(|p| p.weight).sum();
    let left_ms = deadline_ms.saturating_sub(epoch_ms());
    let mut ctx = Ctx {
        inflight: OpenOptions::new().create(true).write(true).truncate(true).open(inflight).expect("inflight file"),
        deadline: Instant::now() + Duration::from_millis(left_ms as u64),
    };
    let mut rng = seed ^ 0x5EED;
    let mut out = std::io::stdout().lock();
    while Instant::now() + Duration::from_millis(200) < ctx.deadline {
        let mut pick = (splitmix(&mut rng) % u64::from(total)) as u32;
        let prop = props.iter().find(|p| {
            if pick < p.weight {
                true
            } else {
                pick -= p.weight;
                false
            }
        });
        let prop = prop.unwrap_or(&props[0]);
        let chunk_seed = splitmix(&mut rng);
        let started = Instant::now();
        let r = (prop.chunk)(&mut ctx, chunk_seed, prop.tests);
        let ms = started.elapsed().as_millis() as u64;
        let mut line = json!({"t": "stats", "property": prop.name, "cases": r.cases, "ms": ms, "labels": r.labels});
        if let Some((sig, msg, case)) = r.finding {
            line = json!({"t": "finding", "property": prop.name, "family": prop.family, "cases": r.cases, "ms": ms, "labels": line["labels"],
                          "signature": sig, "message": msg, "seed": chunk_seed, "case": case});
        }
        if writeln!(out, "{line}").and_then(|_| out.flush()).is_err() {
            break;
        }
    }
    ExitCode::SUCCESS
}

// ---------- supervisor ----------

struct Worker {
    child: Child,
    inflight: PathBuf,
    started: Instant,
    seed: u64,
}

struct Search {
    out: PathBuf,
    binary: String,
    props: Vec<Prop>,
    cases: u64,
    failures: u64,
    per_prop: BTreeMap<String, u64>,
    per_prop_ms: BTreeMap<String, u64>,
    labels: BTreeMap<String, u64>,
    per_sig: BTreeMap<String, u64>,
    crashes: u64,
    hangs: u64,
    restarts: u64,
}

impl Search {
    fn family_of(&self, property: &str) -> &'static str {
        self.props.iter().find(|p| p.name == property).map(|p| p.family).unwrap_or("unknown")
    }

    fn record(&mut self, sig: &str, msg: &str, seed: u64, property: &str, family: &str, case: &Value) {
        self.failures += 1;
        let n = self.per_sig.entry(sig.to_string()).or_insert(0);
        *n += 1;
        if *n > 3 {
            return;
        }
        let dir = self.out.join("findings").join(sig).join(n.to_string());
        if fs::create_dir_all(&dir).is_err() {
            return;
        }
        let finding = json!({"signature": sig, "oracle": family, "message": msg, "seed": seed, "property": property, "binary": self.binary});
        let _ = fs::write(dir.join("finding.json"), serde_json::to_string_pretty(&finding).unwrap_or_default());
        let mut case = case.clone();
        if let Some(obj) = case.as_object_mut() {
            obj.insert("signature".into(), json!(sig));
            obj.insert("binary".into(), json!(self.binary));
        }
        let _ = fs::write(dir.join("case.json"), serde_json::to_string_pretty(&case).unwrap_or_default());
        let hang = if self.binary.ends_with("-asan") { 90 } else { 30 };
        let script = format!(
            "#!/usr/bin/env bash\n# repro.sh <build-dir>: exit 1 = the failure reproduces, 0 = it does not, other = undecided.\n\
             # {sig} ({property})\nB=\"$(cd \"$1\" && pwd)\" || exit 2\ncd \"$(dirname \"$0\")\" || exit 2\n\
             [ -x \"$B/bin/{bin}\" ] || {{ echo \"no $B/bin/{bin} in this build\" >&2; exit 2; }}\n\
             export ASAN_OPTIONS=\"${{ASAN_OPTIONS:-detect_leaks=0:abort_on_error=1}}\"\n\
             BBZ_HANG_S=${{BBZ_HANG_S:-{hang}}} exec \"$B/bin/{bin}\" repro case.json\n",
            bin = self.binary
        );
        let path = dir.join("repro.sh");
        if fs::write(&path, script).is_ok() {
            let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o755));
        }
    }

    /// A worker died or hung: the case in flight is the finding.
    fn crash(&mut self, w: &Worker, kind: &str, detail: &str) {
        let inflight: Option<Value> = fs::read(&w.inflight).ok().and_then(|b| serde_json::from_slice(&b).ok());
        let property = inflight.as_ref().and_then(|v| v["property"].as_str()).unwrap_or("unknown").to_string();
        let family = self.family_of(&property);
        let sig = format!("crash__{kind}__{}", if family == "unknown" { "unknown" } else { family });
        let case = inflight.unwrap_or(json!({"property": property, "input": null}));
        self.record(&sig, &format!("{detail} while running {property}"), w.seed, &property, family, &case);
    }

    fn handle_line(&mut self, line: &str) {
        let Ok(v) = serde_json::from_str::<Value>(line) else { return };
        let property = v["property"].as_str().unwrap_or("").to_string();
        let cases = v["cases"].as_u64().unwrap_or(0);
        self.cases += cases;
        *self.per_prop.entry(property.clone()).or_insert(0) += cases;
        *self.per_prop_ms.entry(property.clone()).or_insert(0) += v["ms"].as_u64().unwrap_or(0);
        if let Some(labels) = v["labels"].as_object() {
            for (k, n) in labels {
                *self.labels.entry(k.clone()).or_insert(0) += n.as_u64().unwrap_or(0);
            }
        }
        if v["t"] == "finding" {
            let family = self.family_of(&property);
            self.record(
                v["signature"].as_str().unwrap_or("harness__error__unknown"),
                v["message"].as_str().unwrap_or(""),
                v["seed"].as_u64().unwrap_or(0),
                &property,
                family,
                &v["case"],
            );
        }
    }

    fn write_stats(&self) {
        let mut stats = json!({
            "cases": self.cases, "failures": self.failures, "crashes": self.crashes, "hangs": self.hangs,
            "worker_restarts": self.restarts, "signatures": self.per_sig,
        });
        let obj = stats.as_object_mut().unwrap();
        for (k, n) in &self.per_prop {
            obj.insert(format!("cases_{k}"), json!(n));
        }
        // worker-milliseconds spent per property, for cases/s
        for (k, n) in &self.per_prop_ms {
            obj.insert(format!("ms_{k}"), json!(n));
        }
        for (k, n) in &self.labels {
            obj.insert(format!("label_{k}"), json!(n));
        }
        let tmp = self.out.join("stats.json.tmp");
        if fs::write(&tmp, serde_json::to_string_pretty(&stats).unwrap_or_default()).is_ok() {
            let _ = fs::rename(&tmp, self.out.join("stats.json"));
        }
    }
}

fn search(seed: u64, secs: u64, out: &Path) -> ExitCode {
    fs::create_dir_all(out).expect("out dir");
    let exe = std::env::current_exe().expect("current exe");
    let binary = exe.file_name().and_then(|s| s.to_str()).unwrap_or("bbz").to_string();
    let n_workers: usize = std::env::var("BBZ_WORKERS").ok().and_then(|s| s.parse().ok()).unwrap_or(2).max(1);
    let hang = Duration::from_secs(std::env::var("BBZ_HANG_S").ok().and_then(|s| s.parse().ok()).unwrap_or(if binary.ends_with("-asan") { 90 } else { 30 }));
    let start = Instant::now();
    let end = start + Duration::from_secs(secs);
    let margin = Duration::from_millis((secs * 1000 / 20).min(5000));
    let worker_end_ms = epoch_ms() + (secs * 1000) as u128 - margin.as_millis();
    let mut s = Search {
        out: out.to_path_buf(),
        binary,
        props: registry(),
        cases: 0,
        failures: 0,
        per_prop: BTreeMap::new(),
        per_prop_ms: BTreeMap::new(),
        labels: BTreeMap::new(),
        per_sig: BTreeMap::new(),
        crashes: 0,
        hangs: 0,
        restarts: 0,
    };
    s.write_stats();
    let (tx, rx) = mpsc::channel::<String>();
    let mut generation = 0u64;
    let spawn = |i: usize, generation: u64| -> Option<Worker> {
        let wseed = seed.wrapping_mul(1_000_003).wrapping_add(i as u64 * 7919).wrapping_add(generation * 104_729);
        let inflight = out.join(format!(".inflight-{i}.json"));
        let _ = fs::remove_file(&inflight);
        let log = OpenOptions::new().create(true).append(true).open(out.join(format!("worker-{i}.log"))).ok()?;
        let mut child = Command::new(&exe)
            .args(["worker", &wseed.to_string(), &worker_end_ms.to_string(), &inflight.to_string_lossy()])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(log))
            .spawn()
            .ok()?;
        let stdout = child.stdout.take()?;
        let tx = tx.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Some(Worker { child, inflight, started: Instant::now(), seed: wseed })
    };
    let mut workers: Vec<Option<Worker>> = (0..n_workers).map(|i| spawn(i, 0)).collect();
    let mut last_stats = Instant::now();
    loop {
        while let Ok(line) = rx.recv_timeout(Duration::from_millis(100)) {
            s.handle_line(&line);
            if last_stats.elapsed() > Duration::from_secs(2) {
                break;
            }
        }
        let time_left = end.saturating_duration_since(Instant::now());
        for i in 0..workers.len() {
            let Some(w) = workers[i].as_mut() else { continue };
            match w.child.try_wait() {
                Ok(Some(status)) => {
                    let w = workers[i].take().unwrap();
                    if !status.success() {
                        s.crashes += 1;
                        let detail = match std::os::unix::process::ExitStatusExt::signal(&status) {
                            Some(sig) => format!("worker killed by signal {sig}"),
                            None => format!("worker exited with {status}"),
                        };
                        // drain what it said before dying
                        while let Ok(line) = rx.try_recv() {
                            s.handle_line(&line);
                        }
                        s.crash(&w, "abort", &detail);
                    }
                    if time_left > margin + Duration::from_secs(2) {
                        generation += 1;
                        s.restarts += 1;
                        workers[i] = spawn(i, generation);
                    }
                }
                Ok(None) => {
                    let touched = fs::metadata(&w.inflight)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.elapsed().ok())
                        .unwrap_or(w.started.elapsed())
                        .min(w.started.elapsed());
                    if touched > hang {
                        let mut w = workers[i].take().unwrap();
                        let _ = w.child.kill();
                        let _ = w.child.wait();
                        s.hangs += 1;
                        s.crash(&w, "hang", &format!("one case ran past {}s", hang.as_secs()));
                        if time_left > margin + Duration::from_secs(2) {
                            generation += 1;
                            s.restarts += 1;
                            workers[i] = spawn(i, generation);
                        }
                    }
                }
                Err(_) => {}
            }
        }
        if last_stats.elapsed() > Duration::from_secs(2) {
            s.write_stats();
            last_stats = Instant::now();
        }
        if workers.iter().all(Option::is_none) || Instant::now() + Duration::from_millis(500) >= end {
            break;
        }
    }
    for w in workers.iter_mut().flatten() {
        let _ = w.child.kill();
        let _ = w.child.wait();
    }
    drop(tx);
    while let Ok(line) = rx.recv_timeout(Duration::from_millis(200)) {
        s.handle_line(&line);
    }
    s.write_stats();
    ExitCode::SUCCESS
}

// ---------- repro ----------

fn load_case(path: &Path) -> Result<(Value, String, String), String> {
    let v: Value = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let property = v["property"].as_str().ok_or("case.json has no property")?.to_string();
    let signature = v["signature"].as_str().unwrap_or("").to_string();
    Ok((v, property, signature))
}

/// Runs the case here; prints `{"signature": ...}` (null when it passed).
fn run_case(path: &Path) -> ExitCode {
    quiet_panics();
    let Ok((v, property, _)) = load_case(path) else { return ExitCode::from(2) };
    let props = registry();
    let Some(prop) = props.iter().find(|p| p.name == property) else { return ExitCode::from(2) };
    match (prop.run_json)(&v["input"]) {
        Ok(Ok(())) => println!("{}", json!({"signature": null})),
        Ok(Err(f)) => println!("{}", json!({"signature": f.sig, "message": f.msg})),
        Err(e) => {
            eprintln!("cannot read the case's input: {e}");
            return ExitCode::from(2);
        }
    }
    ExitCode::SUCCESS
}

/// Exit 1 when the case fails with its signature (for crash__abort, when
/// the process dies; for crash__hang, when it runs past BBZ_HANG_S),
/// 0 when no attempt does, 2 when it cannot decide.
fn repro(path: &Path) -> ExitCode {
    let Ok((_, property, signature)) = load_case(path) else { return ExitCode::from(2) };
    let props = registry();
    let Some(prop) = props.iter().find(|p| p.name == property) else {
        eprintln!("unknown property {property}");
        return ExitCode::from(2);
    };
    let exe = std::env::current_exe().expect("current exe");
    let hang = Duration::from_secs(hang_s());
    let mut undecided = 0;
    for attempt in 0..prop.attempts {
        let Ok(mut child) = Command::new(&exe).arg("run-case").arg(path).stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn() else {
            return ExitCode::from(2);
        };
        let started = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) if started.elapsed() > hang => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(_) => return ExitCode::from(2),
            }
        };
        let mut stdout = String::new();
        if let Some(mut out) = child.stdout.take() {
            let _ = std::io::Read::read_to_string(&mut out, &mut stdout);
        }
        match status {
            None => {
                eprintln!("attempt {attempt}: ran past {}s", hang.as_secs());
                if signature.starts_with("crash__hang__") {
                    return ExitCode::from(1);
                }
                undecided += 1;
            }
            Some(st) if !st.success() => {
                eprintln!("attempt {attempt}: the case process died: {st}");
                if st.code() == Some(2) {
                    return ExitCode::from(2);
                }
                if signature.starts_with("crash__abort__") {
                    return ExitCode::from(1);
                }
                undecided += 1;
            }
            Some(_) => {
                let got: Value = serde_json::from_str(stdout.trim()).unwrap_or(Value::Null);
                let sig = got["signature"].as_str().unwrap_or("");
                eprintln!("attempt {attempt}: {}", if sig.is_empty() { "passed".to_string() } else { format!("{sig}: {}", got["message"]) });
                if !sig.is_empty() && sig == signature {
                    return ExitCode::from(1);
                }
            }
        }
    }
    if undecided == prop.attempts { ExitCode::from(2) } else { ExitCode::SUCCESS }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let num = |i: usize| args.get(i).and_then(|s| s.parse::<u64>().ok());
    match args.get(1).map(String::as_str) {
        Some("search") if args.len() == 5 => search(num(2).unwrap_or(1), num(3).unwrap_or(60), Path::new(&args[4])),
        Some("worker") if args.len() == 5 => {
            worker(num(2).unwrap_or(1), args[3].parse().unwrap_or(0), Path::new(&args[4]))
        }
        Some("repro") if args.len() == 3 => repro(Path::new(&args[2])),
        Some("run-case") if args.len() == 3 => run_case(Path::new(&args[2])),
        Some("gen") if args.len() == 5 => {
            let props = registry();
            match props.iter().find(|p| p.name == args[2]) {
                Some(p) => {
                    (p.samples)(num(3).unwrap_or(1), num(4).unwrap_or(5) as usize);
                    ExitCode::SUCCESS
                }
                None => {
                    eprintln!("unknown property; `bbz list` shows them");
                    ExitCode::from(2)
                }
            }
        }
        Some("list") => {
            for p in registry() {
                println!("{:36} {:14} weight {:2}  tests/chunk {:5}  repro attempts {}", p.name, p.family, p.weight, p.tests, p.attempts);
            }
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("usage: bbz search <seed> <secs> <out> | repro <case.json> | run-case <case.json> | gen <property> <seed> <n> | list");
            ExitCode::from(2)
        }
    }
}
