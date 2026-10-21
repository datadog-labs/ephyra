//! The PostgreSQL code that ephyra-postgresql's inputs reach, read from a
//! server built with clang's source-based coverage
//! (`~/Projects/postgres-cov/inst`: `-O1 -g -fprofile-instr-generate
//! -fcoverage-mapping`, assertions on, the same master commit as the tested
//! build).
//!
//! The server forks its backends without exec, so every backend inherits
//! the postmaster's profile file name (`%m_%p`, the postmaster's pid) and,
//! the `%m` pattern merging profiles, adds its counters to that one file
//! when it exits. What a batch of inputs reaches is then the counters that
//! grew in that file between two snapshots ([`Cluster::snapshot`]), taken
//! once the batch's backend (one session) has exited. Long-lived processes
//! (checkpointer, background writer, WAL writer) flush only at shutdown, so
//! what they run (a CHECKPOINT's work) is not counted.
//!
//! The raw profile ([`Profile`], format version 10): a 16-word header,
//! 64-byte function records (name hash, counter offset, counter count),
//! the counters as 64-bit words, the function names. Counters are mapped to
//! source files ([`function_files`]) through `llvm-cov export` once.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

/// The instrumented binaries.
pub fn bin_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join("Projects/postgres-cov/inst/bin")
}

/// This crate's cluster: `.pg/data`, the socket in `.pg`, profiles in
/// `.pg/prof`.
pub fn pg_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".pg")
}

/// Point ephyra-postgresql's sessions at the instrumented cluster (started
/// with the profile pattern in its environment). Call before any session.
pub fn use_instrumented_cluster() {
    let dir = pg_dir();
    fs::create_dir_all(dir.join("prof")).unwrap();
    std::env::set_var("EPHYRA_PG_BIN", bin_dir());
    std::env::set_var("EPHYRA_PG_DIR", &dir);
    std::env::set_var("LLVM_PROFILE_FILE", dir.join("prof/%m_%p.profraw"));
}

/// A raw profile's function records and counters.
pub struct Profile {
    /// Per function: (name hash, first counter, counter count).
    pub functions: Vec<(u64, usize, usize)>,
    pub counters: Vec<u64>,
    /// The names section (for [`names`]).
    pub names: Vec<u8>,
}

fn word(b: &[u8], k: usize) -> u64 {
    u64::from_le_bytes(b[8 * k..8 * k + 8].try_into().unwrap())
}

const RECORD: usize = 64;

impl Profile {
    /// Parse a version-10 raw profile, or say why not.
    pub fn parse(b: &[u8]) -> Result<Profile, String> {
        if b.len() < 128 || word(b, 0) != 0xff6c_7072_6f66_7281 {
            return Err("not a raw profile".into());
        }
        if word(b, 1) & 0xff != 10 {
            return Err(format!("raw profile version {}", word(b, 1)));
        }
        let binary_ids = word(b, 2) as usize;
        let ndata = word(b, 3) as usize;
        let pad_before = word(b, 4) as usize;
        let ncounters = word(b, 5) as usize;
        let pad_after = word(b, 6) as usize;
        let nbitmap = word(b, 7) as usize;
        let pad_bitmap = word(b, 8) as usize;
        let names_size = word(b, 9) as usize;
        let mut counters_delta = word(b, 10) as i64;
        let data = 128 + binary_ids;
        let cstart = data + ndata * RECORD + pad_before;
        let names_at = cstart + ncounters * 8 + pad_after + nbitmap + pad_bitmap;
        if b.len() < names_at + names_size {
            return Err("truncated raw profile".into());
        }
        let mut functions = Vec::with_capacity(ndata);
        for k in 0..ndata {
            let r = &b[data + k * RECORD..data + (k + 1) * RECORD];
            let name = word(r, 0);
            let ptr = word(r, 2) as i64;
            let n = u32::from_le_bytes(r[48..52].try_into().unwrap()) as usize;
            // Each record's counter pointer is relative to the record; the
            // header's delta is the counters' start relative to the first.
            let offset = (ptr - counters_delta) as usize / 8;
            counters_delta -= RECORD as i64;
            functions.push((name, offset, n));
        }
        let counters = (0..ncounters).map(|k| u64::from_le_bytes(b[cstart + 8 * k..cstart + 8 * k + 8].try_into().unwrap())).collect();
        Ok(Profile { functions, counters, names: b[names_at..names_at + names_size].to_vec() })
    }
}

/// The function names of a names section: ULEB128 uncompressed and
/// compressed lengths, then names separated by `\x01` (uncompressed only).
pub fn names(section: &[u8]) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut i = 0;
    let uleb = |i: &mut usize| -> u64 {
        let (mut v, mut shift) = (0u64, 0);
        while *i < section.len() {
            let byte = section[*i];
            *i += 1;
            v |= u64::from(byte & 0x7f) << shift;
            shift += 7;
            if byte & 0x80 == 0 {
                break;
            }
        }
        v
    };
    while i < section.len() {
        let len = uleb(&mut i) as usize;
        let compressed = uleb(&mut i) as usize;
        if len == 0 && compressed == 0 {
            break;
        }
        if compressed != 0 {
            return Err("compressed function names".into());
        }
        let chunk = &section[i..(i + len).min(section.len())];
        out.extend(chunk.split(|&c| c == 1).map(|n| String::from_utf8_lossy(n).into_owned()));
        i += len;
    }
    Ok(out)
}

/// The name hash a function record carries: the first 8 bytes of the MD5
/// of its name, little-endian.
pub fn name_hash(name: &str) -> u64 {
    u64::from_le_bytes(md5::compute(name.as_bytes()).0[..8].try_into().unwrap())
}

/// The instrumented cluster's merged profile.
pub struct Cluster {
    pub profile: PathBuf,
}

impl Cluster {
    /// The postmaster's profile file (sessions must have started it).
    pub fn find() -> Result<Cluster, String> {
        let pid = fs::read_to_string(pg_dir().join("data/postmaster.pid")).map_err(|e| e.to_string())?;
        let pid = pid.lines().next().unwrap_or("").trim().to_string();
        let suffix = format!("_{pid}.profraw");
        let found = fs::read_dir(pg_dir().join("prof")).map_err(|e| e.to_string())?.filter_map(|e| e.ok()).map(|e| e.path()).find(|p| p.to_string_lossy().ends_with(&suffix));
        found.map(|profile| Cluster { profile }).ok_or_else(|| format!("no profile of postmaster {pid} (was the cluster started with LLVM_PROFILE_FILE?)"))
    }

    /// The merged counters now, read under a shared lock (the runtime merges
    /// under an exclusive one).
    pub fn snapshot(&self) -> Result<Vec<u64>, String> {
        let f = fs::File::open(&self.profile).map_err(|e| e.to_string())?;
        f.lock_shared().map_err(|e| e.to_string())?;
        let bytes = fs::read(&self.profile).map_err(|e| e.to_string())?;
        let _ = f.unlock();
        Ok(Profile::parse(&bytes)?.counters)
    }
}

/// Wait until process `pid` has exited (its profile merged), then a moment
/// for parallel workers that exit after their leader is done with them.
pub fn wait_exit(pid: i32) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline && unsafe { libc::kill(pid, 0) } == 0 {
        std::thread::sleep(Duration::from_millis(2));
    }
    std::thread::sleep(Duration::from_millis(20));
}

/// Counter index to source file, for every function of the profile: from
/// `functions.tsv` (name, file) next to this crate, written by
/// [`write_function_files`].
pub fn function_files(p: &Profile) -> Result<Vec<Option<String>>, String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("functions.tsv");
    let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e} (run `cargo run --release --bin functions`)", path.display()))?;
    let by_hash: HashMap<u64, &str> = text.lines().filter_map(|l| l.split_once('\t')).map(|(n, f)| (name_hash(n), f)).collect();
    let mut out = vec![None; p.counters.len()];
    for &(h, at, n) in &p.functions {
        if let Some(f) = by_hash.get(&h) {
            for c in out.iter_mut().skip(at).take(n) {
                *c = Some(f.to_string());
            }
        }
    }
    Ok(out)
}

/// Write `functions.tsv` from a profile: `llvm-cov export` of the
/// instrumented `postgres` binary gives each function's name and file.
pub fn write_function_files(profraw: &Path) -> Result<usize, String> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let profdata = dir.join("target/one.profdata");
    let run = |cmd: &mut Command| -> Result<Vec<u8>, String> {
        let out = cmd.output().map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).into_owned());
        }
        Ok(out.stdout)
    };
    run(Command::new("xcrun").args(["llvm-profdata", "merge", "-o"]).arg(&profdata).arg(profraw))?;
    let json = run(Command::new("xcrun").args(["llvm-cov", "export", "-skip-expansions", "-instr-profile"]).arg(&profdata).arg(bin_dir().join("postgres")))?;
    let text = String::from_utf8_lossy(&json);
    // Each function object: {"branches":..,"count":..,"filenames":["..."],"name":"..",..}
    let mut rows = Vec::new();
    for chunk in text.split("\"filenames\":[\"").skip(1) {
        let Some((file, rest)) = chunk.split_once('"') else { continue };
        let Some(after) = rest.split_once("\"name\":\"") else { continue };
        let Some((name, _)) = after.1.split_once('"') else { continue };
        let src = file.split("/postgres-cov/").nth(1).unwrap_or(file);
        rows.push(format!("{name}\t{src}"));
    }
    rows.sort();
    rows.dedup();
    fs::write(dir.join("functions.tsv"), rows.join("\n") + "\n").map_err(|e| e.to_string())?;
    Ok(rows.len())
}
