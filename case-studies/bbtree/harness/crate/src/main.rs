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

mod bulk;
mod common;
mod filemap;
mod mmap;
mod store;
mod storethreads;
mod syncalloc;
mod syncthreads;

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
        // bbtree-store, single-threaded with page refusals
        p!("store_default", "store", 6, 150, 1, store::Case, store::case(), store::run::<store::Defaults>),
        p!("store_small", "store", 6, 150, 1, store::Case, store::case(), store::run::<store::Small>),
        p!("store_big_heap", "store", 6, 150, 1, store::Case, store::case(), store::run::<store::BigHeap>),
        p!("store_big_leaves", "store", 6, 150, 1, store::Case, store::case(), store::run::<store::BigLeaves>),
        // bbtree-store shared by threads
        p!("storethreads_default", "storethreads", 6, 100, 20, storethreads::Case, storethreads::case(), storethreads::run::<store::Defaults>),
        p!("storethreads_small", "storethreads", 6, 100, 20, storethreads::Case, storethreads::case(), storethreads::run::<storethreads::SmallInternals>),
        // bbtree::BBTreeMap under page refusals
        p!("syncmap_alloc_u64", "syncmap", 8, 200, 1, syncalloc::Case, syncalloc::case(), syncalloc::run::<u64>),
        p!("syncmap_alloc_string", "syncmap", 8, 200, 1, syncalloc::Case, syncalloc::case(), syncalloc::run::<String>),
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
        // bbtree::BBTreeMap shared by threads, large trees
        p!("syncthreads_u64", "syncthreads", 4, 8, 20, syncthreads::Case, syncthreads::case(), syncthreads::run::<u64, Auto>),
        p!("syncthreads_string", "syncthreads", 4, 8, 20, syncthreads::Case, syncthreads::case(), syncthreads::run::<String, Auto>),
        p!("syncthreads_u64_narrow", "syncthreads", 4, 8, 20, syncthreads::Case, syncthreads::case(), syncthreads::run::<u64, bbtree::Configured<Auto, syncthreads::Narrow>>),
        p!("syncthreads_string_narrow", "syncthreads", 4, 8, 20, syncthreads::Case, syncthreads::case(), syncthreads::run::<String, bbtree::Configured<Auto, syncthreads::Narrow>>),
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
    let props = registry();
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
