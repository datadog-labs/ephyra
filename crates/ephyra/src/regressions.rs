// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The regressions file: failing inputs remembered across runs.
//!
//! When a property fails in the random phase, where the input came from is
//! appended to `<dir>/<name>.jsonl` (one JSON object per line: name, seed,
//! iteration, the input's `Debug` rendering, the shrunk rendering, creation
//! time). The next run of the same property regenerates every stored input
//! before random testing and checks it still renders the same; an entry
//! whose generator has changed is reported as stale rather than silently
//! testing a different input. Commit the directory to keep that guarantee
//! for the whole team.
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

/// One remembered failure.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Entry {
    /// The property's name (see `Config::name`).
    pub name: String,
    /// Seed of the run that found it.
    pub seed: u64,
    /// Generation attempt within that run.
    pub iteration: u64,
    /// `Debug` rendering of the input as generated, used to detect that the
    /// generator has changed.
    pub debug: String,
    /// `Debug` rendering of the shrunk input, for humans reading the file.
    pub shrunk: Option<String>,
    /// Unix time (seconds) when the entry was written.
    pub created: u64,
}

impl Entry {
    pub(crate) fn new(
        name: &str,
        seed: u64,
        iteration: u64,
        debug: String,
        shrunk: Option<String>,
    ) -> Self {
        let created = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        Entry { name: name.to_string(), seed, iteration, debug, shrunk, created }
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
/// are skipped. Names that differ only in punctuation share a file (see
/// [`file_name`]), so entries stored under another name are skipped too.
pub fn load(dir: &Path, name: &str) -> Vec<Entry> {
    // Bytes, not a string: one line that is not UTF-8 costs that line, not the file (#59).
    let Ok(bytes) = fs::read(path_for(dir, name)) else {
        return Vec::new();
    };
    bytes
        .split(|b| *b == b'\n')
        .filter_map(|line| serde_json::from_slice::<Entry>(line).ok())
        .filter(|e| e.name == name)
        .collect()
}

/// Append `entry` unless an entry for the same seed and iteration is
/// already stored. Returns the file's path and whether a line was appended.
pub fn save(dir: &Path, entry: &Entry) -> io::Result<(PathBuf, bool)> {
    let path = path_for(dir, &entry.name);
    if load(dir, &entry.name).iter().any(|e| e.seed == entry.seed && e.iteration == entry.iteration)
    {
        return Ok((path, false));
    }
    fs::create_dir_all(dir)?;
    // A file whose last line has no newline (its last entry deleted by hand) would glue
    // the new entry onto it, and both would be lost (#59).
    let glued = fs::read(&path).is_ok_and(|b| b.last().is_some_and(|c| *c != b'\n'));
    let mut line = if glued { "\n".to_string() } else { String::new() };
    line += &serde_json::to_string(entry).map_err(io::Error::other)?;
    line.push('\n');
    // One write for the line and its newline: appends from parallel tests interleave less.
    let mut file = fs::OpenOptions::new().create(true).append(true).open(&path)?;
    file.write_all(line.as_bytes())?;
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
        let entry = Entry::new("p", 7, 3, "[1]".into(), Some("[]".into()));
        let (path, appended) = save(&dir, &entry).unwrap();
        assert_eq!(path, dir.join("p.jsonl"));
        assert!(appended);
        assert!(!save(&dir, &entry).unwrap().1);
        save(&dir, &Entry::new("p", 8, 0, "[2]".into(), None)).unwrap();
        let loaded = load(&dir, "p");
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].iteration, 3);
        assert_eq!(loaded[1].seed, 8);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_starts_a_new_line_after_a_file_without_a_final_newline() {
        // #59: the new entry was glued onto the last line, and both were lost.
        let dir = temp_dir("newline");
        save(&dir, &Entry::new("p", 1, 0, "[1]".into(), None)).unwrap();
        let path = path_for(&dir, "p");
        let text = fs::read_to_string(&path).unwrap();
        fs::write(&path, text.trim_end_matches('\n')).unwrap();
        save(&dir, &Entry::new("p", 2, 0, "[2]".into(), None)).unwrap();
        let seeds: Vec<u64> = load(&dir, "p").iter().map(|e| e.seed).collect();
        assert_eq!(seeds, vec![1, 2]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_line_that_is_not_utf8_costs_only_itself() {
        // #59: one non-UTF-8 byte made load return nothing.
        let dir = temp_dir("utf8");
        save(&dir, &Entry::new("p", 1, 0, "[1]".into(), None)).unwrap();
        let mut file = fs::OpenOptions::new().append(true).open(path_for(&dir, "p")).unwrap();
        file.write_all(b"{\"name\": \"\xff\"}\n").unwrap();
        assert_eq!(load(&dir, "p").len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn names_sharing_a_file_keep_their_own_entries() {
        let dir = temp_dir("shared");
        assert_eq!(path_for(&dir, "tests::foo"), path_for(&dir, "tests_foo"));
        save(&dir, &Entry::new("tests::foo", 1, 0, "5".into(), None)).unwrap();
        assert!(load(&dir, "tests_foo").is_empty());
        // The same point under the other name is not a duplicate.
        assert!(save(&dir, &Entry::new("tests_foo", 1, 0, "true".into(), None)).unwrap().1);
        assert_eq!(load(&dir, "tests::foo")[0].debug, "5");
        assert_eq!(load(&dir, "tests_foo")[0].debug, "true");
        let _ = fs::remove_dir_all(&dir);
    }
}
