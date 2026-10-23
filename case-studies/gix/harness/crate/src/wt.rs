// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The `status` and `index` oracles: gix's view of the worktree and index against git on the
//! same random repositories (the generator's `Op::Dirty` leaves unstaged edits, intent-to-add
//! entries, skip-worktree/assume-unchanged bits, unmerged stages, untracked/ignored files, index
//! v4, split index and an untracked cache behind).
//!
//! | query | gix | git (answer of record) |
//! |---|---|---|
//! | `#status` | `Repository::status()` (untracked files listed one by one, no renames) | `git status --porcelain=v2 -z -uall --no-renames` |
//! | `#index` | the index entries as `Repository::open_index()` reads them | `git ls-files -s -t -z` |
//! | `#index_rt` | the index written back by `gix_index::File::write_to` | `git ls-files -s -t -z --debug` of the original vs. the copy |
//!
//! Contracts: `Repository::status()` is "most similar to `git status --ignored=no`"
//! (gix/src/status/mod.rs); gix-index reads V2..V4 (decode/header.rs) and "the default options
//! write either index V2 or V3 depending on the content of the entries" (write.rs), readable by
//! git. Documented deviations kept out: a change to a non-file is a type change in gix
//! (gix-status index_as_worktree/types.rs "Deviation"), the tree-cache is written as-is.

use crate::graph::Ans;

/// Status lines: `I <X> path` (HEAD vs index), `W <Y> path` (index vs worktree), `U path`
/// (unmerged; nothing else is listed for an unmerged path), `? path` (untracked).
pub fn gix_status(repo: &gix::Repository) -> Ans {
    use gix::status::{index_worktree::Item as Wt, Item};
    use gix_status_kinds::*;
    // A HEAD that does not resolve to a commit (dangling symref chain, cycle) is its own area.
    // A cycle (`refs/heads/x` -> itself) is a broken repository, not a contract either way.
    let head_err = |e: String| match repo.head_commit() {
        Err(h) if h.to_string().contains("cycle") || e.contains("cycle") => format!("{HEAD_CYCLE}: {e}"),
        Err(_) => format!("head: {e}"),
        Ok(_) => e,
    };
    let iter = repo
        .status(gix::progress::Discard)
        .map_err(|e| e.to_string())?
        .untracked_files(gix::status::UntrackedFiles::Files)
        .index_worktree_rewrites(None)
        .tree_index_track_renames(gix::status::tree_index::TrackRenames::Disabled)
        .index_worktree_submodules(gix::status::Submodule::Given { ignore: gix::submodule::config::Ignore::Dirty, check_dirty: false })
        .into_iter(None)
        .map_err(|e| head_err(e.to_string()))?;
    let mut lines = Vec::new();
    let mut unmerged = std::collections::BTreeSet::new();
    for item in iter {
        let item = item.map_err(|e| head_err(e.to_string()))?;
        match item {
            Item::IndexWorktree(Wt::Modification { rela_path, status, .. }) => {
                let k = match status {
                    EntryStatus::Conflict { .. } => {
                        unmerged.insert(rela_path.to_string());
                        continue;
                    },
                    EntryStatus::Change(Change::Removed) => "D",
                    EntryStatus::Change(Change::Type { .. }) => "T",
                    EntryStatus::Change(Change::Modification { .. } | Change::SubmoduleModification(_)) => "M",
                    EntryStatus::NeedsUpdate(_) => continue,
                    EntryStatus::IntentToAdd => "A",
                };
                lines.push(format!("W {k} {rela_path}"));
            },
            Item::IndexWorktree(Wt::DirectoryContents { entry, .. }) => {
                if matches!(entry.status, gix::dir::entry::Status::Untracked) {
                    lines.push(format!("? {}", entry.rela_path));
                }
            },
            Item::IndexWorktree(Wt::Rewrite { .. }) => return Err("rewrite despite renames off".into()),
            Item::TreeIndex(c) => {
                use gix::diff::index::ChangeRef;
                let c: gix::diff::index::Change = c;
                let (k, p) = match c {
                    ChangeRef::Addition { location, .. } => ("A", location.to_string()),
                    ChangeRef::Deletion { location, .. } => ("D", location.to_string()),
                    ChangeRef::Modification { location, previous_entry_mode, entry_mode, .. } => {
                        let kind = |m: gix::index::entry::Mode| if m.is_submodule() { 2 } else if m == gix::index::entry::Mode::SYMLINK { 1 } else { 0 };
                        (if kind(previous_entry_mode) != kind(entry_mode) { "T" } else { "M" }, location.to_string())
                    },
                    ChangeRef::Rewrite { .. } => return Err("tree-index rewrite despite renames off".into()),
                };
                lines.push(format!("I {k} {p}"));
            },
        }
    }
    Ok(finish(lines, unmerged))
}

/// gix's status error when HEAD is a symbolic-ref cycle (no reference answer).
pub const HEAD_CYCLE: &str = "head-cycle";

mod gix_status_kinds {
    pub use gix::status::plumbing::index_as_worktree::{Change, EntryStatus};
}

fn finish(lines: Vec<String>, unmerged: std::collections::BTreeSet<String>) -> Vec<String> {
    let path_of = |l: &str| l.splitn(3, ' ').last().unwrap_or("").to_string();
    let mut v: Vec<String> = lines
        .into_iter()
        .filter(|l| {
            let p = if l.starts_with("? ") { l[2..].to_string() } else { path_of(l) };
            !unmerged.contains(&p)
        })
        .map(|l| l.trim_end_matches('/').to_string())
        .collect();
    v.extend(unmerged.into_iter().map(|p| format!("U {p}")));
    v.sort();
    v.dedup();
    v
}

pub fn git_status(run: &dyn Fn(&[&str]) -> Option<(bool, String, String)>) -> Option<Ans> {
    let (ok, out, err) = run(&["--no-optional-locks", "status", "--porcelain=v2", "-z", "-uall", "--no-renames", "--ignore-submodules=dirty"])?;
    if !ok {
        return Some(Err(err.trim().to_string()));
    }
    let mut lines = Vec::new();
    let mut unmerged = std::collections::BTreeSet::new();
    for rec in out.split('\0').filter(|r| !r.is_empty()) {
        let f: Vec<&str> = rec.splitn(9, ' ').collect();
        match f[0] {
            "1" => {
                let f: Vec<&str> = rec.splitn(9, ' ').collect();
                let (xy, path) = (f.get(1)?.as_bytes(), *f.get(8)?);
                if xy[0] != b'.' {
                    lines.push(format!("I {} {path}", xy[0] as char));
                }
                if xy[1] != b'.' {
                    lines.push(format!("W {} {path}", xy[1] as char));
                }
            },
            "u" => {
                let f: Vec<&str> = rec.splitn(11, ' ').collect();
                unmerged.insert(f.get(10)?.to_string());
            },
            "?" => lines.push(format!("? {}", &rec[2..])),
            "!" | "#" => {},
            _ => return Some(Err(format!("unparsed status record {rec:?}"))),
        }
    }
    Some(Ok(finish(lines, unmerged)))
}

/// Index entries: `<mode> <id> <stage> <tag> <path>`, tag as `git ls-files -t` (with `-v`:
/// `H` cached, `S` skip-worktree, `M` unmerged, lower case for assume-unchanged) plus `+ita`.
pub fn gix_index(repo: &gix::Repository) -> Ans {
    let index = repo.open_index().map_err(|e| e.to_string())?;
    Ok(entries(&index))
}

fn entries(state: &gix::index::State) -> Vec<String> {
    use gix::index::entry::Flags;
    state
        .entries()
        .iter()
        .map(|e| {
            let mut tag = if e.stage_raw() != 0 { "M" } else if e.flags.contains(Flags::SKIP_WORKTREE) { "S" } else { "H" }.to_string();
            if e.flags.contains(Flags::ASSUME_VALID) {
                tag = tag.to_lowercase();
            }
            if e.flags.contains(Flags::INTENT_TO_ADD) {
                tag.push_str("+ita");
            }
            format!("{:06o} {} {} {tag} {}", e.mode.bits(), e.id, e.stage_raw(), e.path(state))
        })
        .collect()
}

pub fn git_index(run: &dyn Fn(&[&str]) -> Option<(bool, String, String)>) -> Option<Ans> {
    let (ok, out, err) = run(&["ls-files", "-s", "-v", "-z"])?;
    if !ok {
        return Some(Err(err.trim().to_string()));
    }
    // `git ls-files --format` knows no intent-to-add marker; `ls-files --debug` shows its flag
    // bit (1 << 29, `size: 0\tflags: 20004000`).
    let ita = ita_paths(run)?;
    let mut v = Vec::new();
    for rec in out.split('\0').filter(|r| !r.is_empty()) {
        let (tag, rest) = rec.split_once(' ')?;
        let (meta, path) = rest.split_once('\t')?;
        let mut tag = tag.to_string();
        if ita.contains(path) {
            tag.push_str("+ita");
        }
        v.push(format!("{meta} {tag} {path}"));
    }
    Some(Ok(v))
}

fn ita_paths(run: &dyn Fn(&[&str]) -> Option<(bool, String, String)>) -> Option<std::collections::HashSet<String>> {
    let (ok, out, _) = run(&["ls-files", "--debug", "-s"])?;
    if !ok {
        return None;
    }
    let mut set = std::collections::HashSet::new();
    let mut last = None;
    for line in out.lines() {
        if let Some((_, rest)) = line.split_once("flags: ").filter(|_| line.starts_with("  ")) {
            let flags = u32::from_str_radix(rest.trim(), 16).ok()?;
            if flags & 0x2000_0000 != 0 {
                set.insert(last.clone()?);
            }
        } else if !line.starts_with("  ") {
            last = line.split_once('\t').map(|(_, p)| p.to_string());
        }
    }
    Some(set)
}

/// The round-trip file gix writes next to the index.
pub const RT_FILE: &str = "gixfuzz-index-rt";

/// gix reads the index and writes it back (default options) to `.git/gixfuzz-index-rt`;
/// the answer is the listing of what it read.
pub fn gix_index_rt(repo: &gix::Repository) -> Ans {
    let index = repo.open_index().map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    index.write_to(&mut buf, gix::index::write::Options::default()).map_err(|e| e.to_string())?;
    std::fs::write(repo.git_dir().join(RT_FILE), &buf).map_err(|e| e.to_string())?;
    // gix must read back what it wrote, too.
    let back = gix::index::File::at(repo.git_dir().join(RT_FILE), repo.object_hash(), false, Default::default()).map_err(|e| format!("re-read: {e}"))?;
    let (a, b) = (entries(&index), entries(&back));
    if a != b {
        return Err(format!("gix re-reads its own index differently: {:?}", a.iter().zip(&b).find(|(x, y)| x != y)));
    }
    Ok(vec!["same".into()])
}

/// git reads the original and the copy: `ls-files -s -v --debug` (stat data, flags, stages)
/// must be the same. `--debug` flags only show the on-disk bits git keeps in memory.
pub fn git_index_rt(run: &dyn Fn(&[&str]) -> Option<(bool, String, String)>) -> Option<Ans> {
    let list = |env: Option<&str>| {
        let mut args = vec![];
        if let Some(e) = env {
            args.push(e);
        }
        args.extend(["ls-files", "-s", "-v", "--debug"]);
        run(&args)
    };
    // `--debug` prints git's in-memory entry flags; only the on-disk bits (stage, extended,
    // assume-valid, intent-to-add, skip-worktree) are the index's content.
    let norm = |out: String| -> String {
        out.lines()
            .map(|l| match l.split_once("flags: ").filter(|_| l.starts_with("  ")) {
                Some((pre, f)) => match u32::from_str_radix(f.trim(), 16) {
                    Ok(f) => format!("{pre}flags: {:x}", f & 0x6000_f000),
                    Err(_) => l.to_string(),
                },
                None => l.to_string(),
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let (ok, orig, err) = list(None)?;
    let orig = norm(orig);
    if !ok {
        return Some(Err(err.trim().to_string()));
    }
    let rt = format!("@GIT_INDEX_FILE=.git/{RT_FILE}");
    let (ok, copy, err) = list(Some(&rt))?;
    if !ok {
        return Some(Ok(vec![format!("git cannot read gix's index: {}", err.trim())]));
    }
    let copy = norm(copy);
    if orig == copy {
        return Some(Ok(vec!["same".into()]));
    }
    let (a, b): (Vec<&str>, Vec<&str>) = (orig.lines().collect(), copy.lines().collect());
    let i = a.iter().zip(&b).position(|(x, y)| x != y).unwrap_or(a.len().min(b.len()));
    Some(Ok(vec![format!("differs at line {i}: original {:?}, gix-written {:?}", a.get(i), b.get(i))]))
}

fn differing<'a>(gix: &'a Ans, git: &'a Ans) -> Vec<&'a String> {
    match (gix, git) {
        (Ok(a), Ok(b)) => a.iter().filter(|l| !b.contains(l)).chain(b.iter().filter(|l| !a.contains(l))).collect(),
        _ => vec![],
    }
}

/// The most specific kind of status line that differs.
pub fn status_area(gix: &Ans, git: &Ans) -> &'static str {
    if let Err(e) = gix {
        if e.starts_with("head: ") {
            return "unresolvable_head";
        }
    }
    let d = differing(gix, git);
    // A staged deletion under an intent-to-add entry of the same path.
    if let Ok(g) = git {
        if d.iter().any(|l| l.strip_prefix("I D ").is_some_and(|p| g.contains(&format!("W A {p}")))) {
            return "staged_delete_ita";
        }
    }
    let has = |pre: &str| d.iter().any(|l| l.starts_with(pre));
    for (pre, area) in [
        ("U ", "unmerged"),
        ("W A", "intent_to_add"),
        ("I A", "staged_add"),
        ("? ", "untracked"),
        ("W T", "worktree_typechange"),
        ("I T", "staged_typechange"),
        ("W D", "worktree_deleted"),
        ("I D", "staged_delete"),
        ("W M", "worktree_modified"),
        ("I M", "staged_modify"),
    ] {
        if has(pre) {
            return area;
        }
    }
    "status"
}

pub fn index_area(gix: &Ans, git: &Ans) -> &'static str {
    let d = differing(gix, git);
    let field = |l: &str, i: usize| l.split(' ').nth(i).unwrap_or("").to_string();
    if d.iter().any(|l| field(l, 2) != "0") {
        "entries_stage"
    } else if d.iter().any(|l| field(l, 3).contains("ita")) {
        "entries_ita"
    } else if d.iter().any(|l| field(l, 3) != "H") {
        "entries_flags"
    } else {
        "entries"
    }
}

/// git leaves out the deletion of a tracked path under a symlink loop (`d -> d/c` with `d/c`
/// tracked: its lstat fails with ELOOP, not ENOENT, and git reports nothing), while the path is
/// gone from the worktree all the same. True when every difference is a gix `W D p` that git
/// lacks and `p` has such a parent.
pub fn git_misses_deletion_under_symlink_loop(repo_dir: &std::path::Path, gix: &Ans, git: &Ans) -> bool {
    let (Ok(a), Ok(b)) = (gix, git) else { return false };
    let only_gix: Vec<&String> = a.iter().filter(|l| !b.contains(l)).collect();
    let loops = |p: &str| {
        let mut cur = std::path::PathBuf::new();
        let comps: Vec<&str> = p.split('/').collect();
        comps[..comps.len().saturating_sub(1)].iter().any(|c| {
            cur.push(c);
            let full = repo_dir.join(&cur);
            full.symlink_metadata().is_ok_and(|m| m.file_type().is_symlink()) && std::fs::metadata(&full).is_err_and(|e| e.raw_os_error() == Some(40))
        })
    };
    b.iter().all(|l| a.contains(l)) && !only_gix.is_empty() && only_gix.iter().all(|l| l.strip_prefix("W D ").is_some_and(loops))
}

/// git-update-index(1): with the assume-unchanged bit "the user promises not to change the file
/// and allows Git to assume that the working tree file matches what is recorded in the index".
/// gix keeps that promise for the entry's type too (72134f560, gix-dir test
/// `assume_unchanged_submodule_replaced_with_symlink_is_hidden`: "assume-unchanged entries
/// should use index type information and stay hidden"), so it never reads a directory that
/// replaced such a file; git lists its contents with `-uall` (and hides them without it).
/// True when every difference is a git `? p` that gix lacks and a parent of `p` is an
/// assume-unchanged index entry.
pub fn git_untracked_under_assume_unchanged(run: &dyn Fn(&[&str]) -> Option<(bool, String, String)>, gix: &Ans, git: &Ans) -> bool {
    let (Ok(a), Ok(b)) = (gix, git) else { return false };
    let only_git: Vec<&String> = b.iter().filter(|l| !a.contains(l)).collect();
    if only_git.is_empty() || !a.iter().all(|l| b.contains(l)) {
        return false;
    }
    let Some((true, out, _)) = run(&["ls-files", "-v", "-z"]) else { return false };
    // `-v` tags assume-unchanged entries with a lower-case letter.
    let assumed: std::collections::HashSet<&str> = out
        .split('\0')
        .filter_map(|r| r.split_once(' '))
        .filter(|(tag, _)| tag.chars().all(|c| c.is_ascii_lowercase()))
        .map(|(_, p)| p)
        .collect();
    only_git.iter().all(|l| {
        l.strip_prefix("? ").is_some_and(|p| {
            p.match_indices('/').any(|(i, _)| assumed.contains(&p[..i]))
        })
    })
}
