// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The run's records, append-only JSON lines that concurrent processes can
//! write (each line is one `write` on a file opened for appending, and stays
//! well under 4 KB), plus the header:
//!
//! - `run.json` (schema `ephyra.run.v1`), written once at the run's first
//!   start: the run id, campaign and work tag; the seed and configuration;
//!   the host; the version (a content hash of every source file of ephyra,
//!   ephyra-sqlite and sqlite-coverage, each repository's commit and
//!   whether it is dirty, the vendored SQLite amalgamation's hash, and the
//!   runner binary's hash); and the target (the linked SQLite's source id).
//! - `events.jsonl`: `{"ts", "run", "kind", ..}` per start, stop, crash,
//!   hang, memory kill, give-up, finding and saved timeout. A finding's
//!   names its example directory (`example`, null past the signature's
//!   examples) and whether that has an `unshrunk/` copy of the input before
//!   shrinking (`unshrunk`, a statement error's: [`crate::findings`]).
//! - `snapshots.jsonl`: `{"ts", "run", "kind": "worker"|"total", ..}`, a
//!   worker's totals (by oracle too) every 30 seconds, the supervisor's sum
//!   every minute.
//!
//! Content hashes are FNV-1a (64 bits, [`crate::content`]): identities,
//! not security.

use std::{
    fs::{self, OpenOptions},
    io::Write as _,
    path::Path,
    process::Command,
};

use crate::{
    content::{self, component_hash, components, file_hash, version_id},
    now,
};

/// A JSON string literal.
pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Append one JSON line, `{"ts": .., "run": .., <fields>}`, to `path`.
pub fn append(path: &Path, run: &str, fields: &str) {
    let mut line = format!("{{\"ts\":{},\"run\":{}", now(), esc(run));
    if !fields.is_empty() {
        line.push(',');
        line.push_str(fields);
    }
    line.push_str("}\n");
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = f.write_all(line.as_bytes());
    }
}

/// An event in `D/events.jsonl` (and a readable line in `D/events.log`).
pub fn event(dir: &Path, run: &str, kind: &str, fields: &str, text: &str) {
    let f = if fields.is_empty() { format!("\"kind\":{}", esc(kind)) } else { format!("\"kind\":{},{fields}", esc(kind)) };
    append(&dir.join("events.jsonl"), run, &f);
    crate::event(dir, text);
}

/// A component's content hash (its sources' relative paths and bytes) and,
/// when it is in a git repository, the commit and whether its files differ
/// from it.
fn component(name: &str, dir: &Path) -> (String, u64) {
    let (h, files) = component_hash(dir);
    let git = |args: &[&str]| {
        Command::new("git").arg("-C").arg(dir).args(args).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    let commit = git(&["rev-parse", "HEAD"]);
    let dirty = git(&["status", "--porcelain", "--", "."]).map(|s| !s.is_empty());
    let json = format!(
        "{}:{{\"hash\":\"{h:016x}\",\"files\":{},\"commit\":{},\"dirty\":{}}}",
        esc(name),
        files,
        commit.as_deref().map_or("null".into(), esc),
        dirty.map_or("null".into(), |d| d.to_string())
    );
    (json, h)
}

/// The linked SQLite's source id (`sqlite3_sourceid()`).
pub fn sqlite_source_id() -> String {
    // SAFETY: sqlite3_sourceid returns a pointer to a static NUL-terminated string.
    unsafe { std::ffi::CStr::from_ptr(rusqlite::ffi::sqlite3_sourceid()) }.to_string_lossy().into_owned()
}

/// The version of the sources on disk: `(json, id)`, id a short hash of
/// every component's hash ([`content::version_id`]). The JSON also has the
/// version the binary was built from (`built`) and the binary's hash.
pub fn version() -> (String, String) {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    let items: Vec<String> = components(here).iter().map(|(name, dir)| component(name, dir).0).collect();
    let amalgamation = file_hash(&content::amalgamation(here));
    let binary = std::env::current_exe().ok().and_then(|p| file_hash(&p));
    let id = version_id(here);
    let json = format!(
        "{{\"id\":\"{id}\",\"built\":\"{}\",\"components\":{{{}}},\"sqlite_amalgamation\":{},\"binary\":{}}}",
        built_version(),
        items.join(","),
        amalgamation.map_or("null".into(), |h| format!("\"{h:016x}\"")),
        binary.map_or("null".into(), |h| format!("\"{h:016x}\""))
    );
    (json, id)
}

/// The version of the sources the binary was built from.
pub fn built_version() -> &'static str {
    env!("RUNNER_BUILT_VERSION")
}

/// The run's identity, as workers and metrics carry it.
#[derive(Clone)]
pub struct Identity {
    pub run: String,
    pub campaign: String,
    pub tag: String,
    pub version: String,
    /// The SQLite check-in, the source id's hash (12 characters).
    pub target: String,
}

impl Identity {
    /// From `run.json`, which [`header`] wrote.
    pub fn load(dir: &Path) -> Option<Identity> {
        let text = fs::read_to_string(dir.join("run.json")).ok()?;
        let field = |key: &str| {
            let at = text.find(&format!("\"{key}\":\""))? + key.len() + 4;
            Some(text[at..].split('"').next()?.to_string())
        };
        Some(Identity { run: field("run_id")?, campaign: field("campaign")?, tag: field("work_tag")?, version: field("version_id")?, target: field("target_id")? })
    }

    /// DogStatsD tags.
    pub fn tags(&self) -> String {
        format!("run:{},campaign:{},work_tag:{},version:{},target:sqlite-{}", self.run, self.campaign, self.tag, self.version, self.target)
    }
}

/// Write `D/run.json` unless it exists; the identity either way, and whether
/// the running binary's version differs from the header's.
pub fn header(dir: &Path, campaign: &str, tag: &str, seed: u64, config: &str) -> (Identity, bool) {
    let (version_json, version_id) = version();
    let source = sqlite_source_id();
    let target_id = source.split_whitespace().nth(2).map_or("unknown".to_string(), |h| h.chars().take(12).collect());
    if let Some(id) = Identity::load(dir) {
        return (id.clone(), id.version != version_id);
    }
    let run = format!("{}-{}", dir.file_name().and_then(|n| n.to_str()).unwrap_or("run"), now());
    let host = Command::new("hostname").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    let json = format!(
        "{{\"schema\":\"ephyra.run.v1\",\"run_id\":{},\"started\":{},\"campaign\":{},\"work_tag\":{},\"seed\":{seed},\"host\":{},\"config\":{config},\"version_id\":\"{version_id}\",\"version\":{version_json},\"target_id\":\"{target_id}\",\"target\":{{\"name\":\"sqlite\",\"source_id\":{}}}}}\n",
        esc(&run),
        now(),
        esc(campaign),
        esc(tag),
        esc(&host),
        esc(&source)
    );
    crate::write_atomic(&dir.join("run.json"), &json);
    (Identity { run, campaign: campaign.into(), tag: tag.into(), version: version_id, target: target_id }, false)
}

#[cfg(test)]
mod tests {
    use super::{esc, version};

    #[test]
    fn strings_escape_as_json() {
        assert_eq!(esc("a\"b\\c\nd\u{1}"), "\"a\\\"b\\\\c\\nd\\u0001\"");
    }

    #[test]
    fn the_version_covers_three_components_and_is_stable() {
        let (json, id) = version();
        for name in ["ephyra", "ephyra-sqlite", "sqlite-coverage"] {
            assert!(json.contains(&format!("\"{name}\":{{\"hash\"")), "{json}");
        }
        assert_eq!(version().1, id);
    }

    /// The test binary is built from the sources on disk.
    #[test]
    fn the_built_version_is_the_version_on_disk() {
        assert_eq!(super::built_version(), version().1);
    }
}
