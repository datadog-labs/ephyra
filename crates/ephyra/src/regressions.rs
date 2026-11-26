// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The regressions file: failing inputs remembered across runs.
//!
//! When a property fails, where the input came from is appended to
//! `<dir>/<name>.jsonl` (one JSON object per line: name, seed, iteration,
//! the iterations that mutated it if any, the input's `Debug` rendering, the
//! shrunk rendering, creation time). The next run of the same property
//! regenerates every stored input before random testing and checks it still
//! renders the same (in any order, when it renders in another order every
//! time, as hash collections do); an entry whose generator or mutator has
//! changed is reported as stale rather than silently testing a different
//! input. Commit the directory to keep that guarantee for the whole team.
//!
//! Requires the default `json` feature.

use std::{
    fs,
    io::{
        self,
        Write,
    },
    path::{
        Path,
        PathBuf,
    },
    time::{
        SystemTime,
        UNIX_EPOCH,
    },
};

use crate::config::ReplayPoint;

/// One remembered failure.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Entry {
    /// The property's name (see `Config::name`).
    pub name: String,
    /// Seed of the run that found it.
    pub seed: u64,
    /// Generation attempt within that run.
    pub iteration: u64,
    /// The attempts of that run that mutated the generated input into the
    /// failing one, in order; empty when it failed as generated.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mutations: Vec<u64>,
    /// `Debug` rendering of the input as generated (and mutated), used to
    /// detect that the generator has changed.
    pub debug: String,
    /// `Debug` rendering of the shrunk input, for humans reading the file.
    pub shrunk: Option<String>,
    /// Unix time (seconds) when the entry was written.
    pub created: u64,
}

impl Entry {
    pub(crate) fn new(
        name: &str,
        point: ReplayPoint,
        mutations: Vec<u64>,
        debug: String,
        shrunk: Option<String>,
    ) -> Self {
        let created = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let (name, seed, iteration) = (name.to_string(), point.seed, point.iteration);
        Entry { name, seed, iteration, mutations, debug, shrunk, created }
    }
}

/// Turn a property name (often `module::test_fn`) into a file name.
pub fn file_name(name: &str) -> String {
    format!("{}.jsonl", crate::file_stem(name))
}

/// The file that holds entries for `name` under `dir`.
pub fn path_for(dir: &Path, name: &str) -> PathBuf {
    dir.join(file_name(name))
}

/// Load the entries stored for `name`. Missing files and unparsable lines
/// are skipped.
pub fn load(dir: &Path, name: &str) -> Vec<Entry> {
    let Ok(text) = fs::read_to_string(path_for(dir, name)) else {
        return Vec::new();
    };
    text.lines().filter_map(|line| serde_json::from_str::<Entry>(line).ok()).collect()
}

/// Append `entry` unless an entry for the same seed, iteration and mutations
/// is already stored. Returns the file's path and whether a line was appended.
pub fn save(dir: &Path, entry: &Entry) -> io::Result<(PathBuf, bool)> {
    let path = path_for(dir, &entry.name);
    let same = |e: &Entry| {
        (e.seed, e.iteration) == (entry.seed, entry.iteration) && e.mutations == entry.mutations
    };
    if load(dir, &entry.name).iter().any(same) {
        return Ok((path, false));
    }
    fs::create_dir_all(dir)?;
    let mut file = fs::OpenOptions::new().create(true).append(true).open(&path)?;
    let line = serde_json::to_string(entry).map_err(io::Error::other)?;
    writeln!(file, "{line}")?;
    Ok((path, true))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ephyra-regressions-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn file_names_are_sanitised() {
        assert_eq!(
            file_name("runner::tests::passing_property"),
            "runner_tests_passing_property.jsonl"
        );
        assert_eq!(file_name("  "), "unnamed.jsonl");
        assert_eq!(file_name("a/b c"), "a_b_c.jsonl");
    }

    #[test]
    fn save_then_load_dedupes_by_point() {
        let dir = temp_dir("dedupe");
        assert!(load(&dir, "p").is_empty());
        let point = |seed, iteration| ReplayPoint { seed, iteration };
        let entry = Entry::new("p", point(7, 3), vec![], "[1]".into(), Some("[]".into()));
        let (path, appended) = save(&dir, &entry).unwrap();
        assert_eq!(path, dir.join("p.jsonl"));
        assert!(appended);
        assert!(!save(&dir, &entry).unwrap().1);
        save(&dir, &Entry::new("p", point(8, 0), vec![], "[2]".into(), None)).unwrap();
        let mutant = Entry::new("p", point(7, 3), vec![5, 9], "[3]".into(), None);
        assert!(save(&dir, &mutant).unwrap().1, "another input than the one it comes from");
        assert!(!save(&dir, &mutant).unwrap().1);
        let loaded = load(&dir, "p");
        assert_eq!(loaded.len(), 3);
        assert_eq!(loaded[0].iteration, 3);
        assert_eq!(loaded[1].seed, 8);
        assert_eq!(loaded[2], mutant);
        // Files without mutations are written and read as before.
        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(text.matches("mutations").count(), 1, "{text}");
        let _ = fs::remove_dir_all(&dir);
    }
}
