// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Content hashes of the runner's sources, shared by the build script (the
//! version the binary is built from) and [`crate::records`] (the version on
//! disk). FNV-1a (64 bits): identities, not security.

use std::{
    fs,
    path::{Path, PathBuf},
};

pub const FNV0: u64 = 0xcbf2_9ce4_8422_2325;

pub fn fnv(h: &mut u64, bytes: &[u8]) {
    for b in bytes {
        *h ^= u64::from(*b);
        *h = h.wrapping_mul(0x100_0000_01b3);
    }
}

/// FNV-1a of a file's bytes.
pub fn file_hash(path: &Path) -> Option<u64> {
    let bytes = fs::read(path).ok()?;
    let mut h = FNV0;
    fnv(&mut h, &bytes);
    Some(h)
}

/// The components, by their directory relative to sqlite-coverage's.
pub fn components(here: &Path) -> [(&'static str, PathBuf); 3] {
    [("ephyra", here.join("../../crates/ephyra")), ("ephyra-sqlite", here.join("../generator")), ("sqlite-coverage", here.to_path_buf())]
}

/// The vendored SQLite amalgamation.
pub fn amalgamation(here: &Path) -> PathBuf {
    here.join("../vendor/libsqlite3-sys-trunk/sqlite3/sqlite3.c")
}

/// The `.rs` files and Cargo manifests under `dir` (`target` skipped), sorted.
pub fn sources(dir: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for e in entries.filter_map(Result::ok) {
            let p = e.path();
            let name = e.file_name();
            if p.is_dir() {
                if name != "target" && name != ".git" {
                    walk(&p, out);
                }
            } else if p.extension().is_some_and(|x| x == "rs") || name == "Cargo.toml" || name == "Cargo.lock" {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, &mut out);
    out.sort();
    out
}

/// A component's content hash: its sources' relative paths and bytes.
pub fn component_hash(dir: &Path) -> (u64, usize) {
    let files = sources(dir);
    let mut h = FNV0;
    for f in &files {
        fnv(&mut h, f.strip_prefix(dir).unwrap_or(f).to_string_lossy().as_bytes());
        fnv(&mut h, &file_hash(f).unwrap_or(0).to_le_bytes());
    }
    (h, files.len())
}

/// The version id: a short hash of every component's hash and the
/// amalgamation's.
pub fn version_id(here: &Path) -> String {
    let mut id = FNV0;
    for (_, dir) in components(here) {
        fnv(&mut id, &component_hash(&dir).0.to_le_bytes());
    }
    fnv(&mut id, &file_hash(&amalgamation(here)).unwrap_or(0).to_le_bytes());
    format!("{:012x}", id >> 16)
}
