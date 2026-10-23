// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The run's records, append-only JSON lines that concurrent processes can
//! write (each line is one `write` on a file opened for appending), plus the
//! header:
//!
//! - `run.json` (schema `ephyra.run.v1`, as sqlite-coverage's runner),
//!   written once at the run's first start: the run id, campaign and work
//!   tag; the seed and configuration; the host; the version (a content hash
//!   of every source file of ephyra, ephyra-postgresql and
//!   postgresql-coverage, each repository's commit and whether it is dirty,
//!   and the runner binary's hash); and the target (`"name":"postgresql"`,
//!   the server's `postgres -V`, the commit of the checkout its binaries
//!   were installed from, `pg_config --configure`).
//! - `events.jsonl`: `{"ts", "run", "kind", ..}` per start, stop, finding,
//!   crash, hang, memory kill, server restart, give-up.
//! - `snapshots.jsonl`: `{"ts", "run", "kind": "worker"|"total", ..}`, a
//!   worker's totals (by oracle too) every 30 seconds, the supervisor's sum
//!   every minute.

use std::{
    fs::{self, OpenOptions},
    io::Write as _,
    path::{Path, PathBuf},
    process::Command,
};

use crate::{
    content::{component_hash, components, file_hash, version_id},
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

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    Command::new("git").arg("-C").arg(dir).args(args).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

/// A component's content hash and, when it is in a git repository, the
/// commit and whether its files differ from it.
fn component(name: &str, dir: &Path) -> String {
    let (h, files) = component_hash(dir);
    let commit = git(dir, &["rev-parse", "HEAD"]);
    let dirty = git(dir, &["status", "--porcelain", "--", "."]).map(|s| !s.is_empty());
    format!(
        "{}:{{\"hash\":\"{h:016x}\",\"files\":{},\"commit\":{},\"dirty\":{}}}",
        esc(name),
        files,
        commit.as_deref().map_or("null".into(), esc),
        dirty.map_or("null".into(), |d| d.to_string())
    )
}

/// The version of the sources on disk: `(json, id)`.
pub fn version() -> (String, String) {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    let items: Vec<String> = components(here).iter().map(|(name, dir)| component(name, dir)).collect();
    let binary = std::env::current_exe().ok().and_then(|p| file_hash(&p));
    let id = version_id(here);
    let json = format!(
        "{{\"id\":\"{id}\",\"built\":\"{}\",\"components\":{{{}}},\"binary\":{}}}",
        built_version(),
        items.join(","),
        binary.map_or("null".into(), |h| format!("\"{h:016x}\""))
    );
    (json, id)
}

/// The version of the sources the binary was built from.
pub fn built_version() -> &'static str {
    env!("RUNNER_BUILT_VERSION")
}

/// The server under test: `(json, id)`, id the first 12 characters of the
/// commit of the checkout the binaries in `bin` were installed from (`bin`
/// is `<checkout>/inst/bin` by convention; `pg_config --configure` and
/// `postgres -V` are recorded too), or `unknown`.
pub fn target(bin: &Path) -> (String, String) {
    let out = |cmd: &str, args: &[&str]| {
        Command::new(bin.join(cmd)).args(args).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    let version = out("postgres", &["-V"]);
    let configure = out("pg_config", &["--configure"]);
    let checkout: Option<PathBuf> = bin.parent().and_then(Path::parent).map(Path::to_path_buf);
    let commit = checkout.as_deref().and_then(|c| git(c, &["rev-parse", "HEAD"]));
    let id = commit.as_deref().map_or("unknown".to_string(), |c| c.chars().take(12).collect());
    let json = format!(
        "{{\"name\":\"postgresql\",\"bin\":{},\"version\":{},\"commit\":{},\"configure\":{}}}",
        esc(&bin.to_string_lossy()),
        version.as_deref().map_or("null".into(), esc),
        commit.as_deref().map_or("null".into(), esc),
        configure.as_deref().map_or("null".into(), esc)
    );
    (json, id)
}

/// The run's identity, as workers and metrics carry it.
#[derive(Clone)]
pub struct Identity {
    pub run: String,
    pub campaign: String,
    pub tag: String,
    pub version: String,
    /// The PostgreSQL commit (12 characters).
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

    /// A placeholder for a worker started without a header.
    pub fn unknown() -> Identity {
        Identity { run: "unknown".into(), campaign: "unknown".into(), tag: "untagged".into(), version: "unknown".into(), target: "unknown".into() }
    }

    /// DogStatsD tags.
    pub fn tags(&self) -> String {
        format!("run:{},campaign:{},work_tag:{},version:{},target:postgresql-{}", self.run, self.campaign, self.tag, self.version, self.target)
    }
}

/// Write `D/run.json` unless it exists; the identity either way, and whether
/// the running binary's version differs from the header's.
pub fn header(dir: &Path, campaign: &str, tag: &str, seed: u64, config: &str, bin: &Path) -> (Identity, bool) {
    let (version_json, version_id) = version();
    if let Some(id) = Identity::load(dir) {
        return (id.clone(), id.version != version_id);
    }
    let (target_json, target_id) = target(bin);
    let run = format!("{}-{}", dir.file_name().and_then(|n| n.to_str()).unwrap_or("run"), now());
    let host = Command::new("hostname").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    let json = format!(
        "{{\"schema\":\"ephyra.run.v1\",\"run_id\":{},\"started\":{},\"campaign\":{},\"work_tag\":{},\"seed\":{seed},\"host\":{},\"config\":{config},\"version_id\":\"{version_id}\",\"version\":{version_json},\"target_id\":\"{target_id}\",\"target\":{target_json}}}\n",
        esc(&run),
        now(),
        esc(campaign),
        esc(tag),
        esc(&host)
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
        for name in ["ephyra", "ephyra-postgresql", "postgresql-coverage"] {
            assert!(json.contains(&format!("\"{name}\":{{\"hash\"")), "{json}");
        }
        assert_eq!(version().1, id);
    }
}
