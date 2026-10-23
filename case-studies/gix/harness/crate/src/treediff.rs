// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The `treediff` oracle: gix tree-to-tree diffs against `git diff-tree` on the same random
//! repositories as the revspec oracle. Queries are `#diff <mode> <A> <B>` (A, B: commit ids, or
//! `<B>~1` for B's first parent; the diff is between their trees).
//!
//! | mode | gix | git (answer of record) |
//! |---|---|---|
//! | `nore` | `Repository::diff_tree_to_tree`, rewrites off | `git diff-tree -r -z --no-abbrev --no-renames A B` |
//! | `exact` | rewrites with `percentage: None` (exact renames only), no copies | `git diff-tree -r -z --no-abbrev -M100% --no-rename-empty A B` |
//!
//! Contract: git is the reference (AGENTS.md); `Change` in gix-diff mirrors git's raw diff
//! (addition, deletion, modification, rewrite). Trees are not listed (`-r` without `-t`); a
//! blob <-> tree change at one path is a deletion plus additions, as in git.
//!
//! Exact renames are only compared when they are unambiguous: git prefers a source with the
//! same basename among several identical ones, gix takes the first found
//! (gix-diff/src/rewrites/tracker.rs:1-7 documents that its candidate choice differs).
//! So when a blob id occurs in more than one deletion or more than one addition, there is no
//! reference answer.
//!
//! Empty blobs: gix's default `Rewrites::track_empty` is `false`, documented in
//! gix-diff/src/lib.rs:41-43 ("If `false`, they do not participate in rename tracking. Leaving
//! this off usually leads to better results as empty files don't have a unique-enough
//! identity."), while git renames empty files unless told `--no-rename-empty`. git is asked for
//! the same setting, and empty blobs never make a case ambiguous.

use crate::graph::{Ans, Query};

/// git's answer could not be used as a reference (ambiguous exact renames).
pub const AMBIGUOUS: &str = "?ambiguous-renames";

fn mode_str(m: gix::object::tree::EntryMode) -> Option<String> {
    use gix::object::tree::EntryKind;
    match m.kind() {
        EntryKind::Tree => None,
        k => Some(format!("{:06o}", k as u16)),
    }
}

/// One canonical line: `<old mode> <new mode> <old id> <new id> <path>` or, for a rename,
/// `R <old mode> <new mode> <old id> <new id> <src> -> <dst>`.
pub fn gix_answer(repo: &gix::Repository, q: &Query<'_>) -> Ans {
    let w = &q.words;
    let mode = w.first().copied().unwrap_or("nore");
    let tree = |s: &str| -> Result<gix::Tree<'_>, String> {
        let id = match s.strip_suffix("~1") {
            Some(c) => {
                let c = repo.find_commit(gix::ObjectId::from_hex(c.as_bytes()).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
                let first = c.parent_ids().next().map(|p| p.detach());
                first.ok_or("no parent")?
            },
            None => gix::ObjectId::from_hex(s.as_bytes()).map_err(|e| e.to_string())?,
        };
        repo.find_commit(id).map_err(|e| e.to_string())?.tree().map_err(|e| e.to_string())
    };
    let (a, b) = (tree(w.get(1).ok_or("args")?)?, tree(w.get(2).ok_or("args")?)?);
    let rewrites = match mode {
        "exact" => Some(gix::diff::Rewrites { copies: None, percentage: None, ..Default::default() }),
        _ => None,
    };
    let opts = gix::diff::Options::default().with_rewrites(rewrites);
    let changes = repo.diff_tree_to_tree(&a, &b, opts).map_err(|e| e.to_string())?;
    let null = repo.object_hash().null().to_string();
    let mut out = Vec::new();
    use gix::object::tree::diff::ChangeDetached as C;
    for c in changes {
        match c {
            C::Addition { location, entry_mode, id, .. } => {
                if let Some(m) = mode_str(entry_mode) {
                    out.push(format!("000000 {m} {null} {id} {location}"));
                }
            },
            C::Deletion { location, entry_mode, id, .. } => {
                if let Some(m) = mode_str(entry_mode) {
                    out.push(format!("{m} 000000 {id} {null} {location}"));
                }
            },
            C::Modification { location, previous_entry_mode, previous_id, entry_mode, id } => {
                match (mode_str(previous_entry_mode), mode_str(entry_mode)) {
                    (Some(pm), Some(m)) => out.push(format!("{pm} {m} {previous_id} {id} {location}")),
                    (Some(pm), None) => out.push(format!("{pm} 000000 {previous_id} {null} {location}")),
                    (None, Some(m)) => out.push(format!("000000 {m} {null} {id} {location}")),
                    (None, None) => {},
                }
            },
            C::Rewrite { source_location, source_entry_mode, source_id, entry_mode, id, location, copy, .. } => {
                let (Some(sm), Some(m)) = (mode_str(source_entry_mode), mode_str(entry_mode)) else {
                    // A directory "rename" (gix tracks those internally); its files are listed too.
                    continue;
                };
                let k = if copy { "C" } else { "R" };
                out.push(format!("{k} {sm} {m} {source_id} {id} {source_location} -> {location}"));
            },
        }
    }
    out.sort();
    Ok(out)
}

fn parse_raw(out: &str) -> Option<Vec<String>> {
    let mut v = Vec::new();
    let mut it = out.split('\0').filter(|s| !s.is_empty()).peekable();
    while let Some(head) = it.next() {
        let head = head.trim_start_matches('\n').strip_prefix(':')?;
        let f: Vec<&str> = head.split(' ').collect();
        if f.len() != 5 {
            return None;
        }
        let path = it.next()?;
        match f[4].chars().next()? {
            k @ ('R' | 'C') => {
                let dst = it.next()?;
                v.push(format!("{k} {} {} {} {} {path} -> {dst}", f[0], f[1], f[2], f[3]));
            },
            _ => v.push(format!("{} {} {} {} {path}", f[0], f[1], f[2], f[3])),
        }
    }
    v.sort();
    Some(v)
}

fn is_empty_blob(id: &str) -> bool {
    gix::ObjectId::from_hex(id.as_bytes()).is_ok_and(|id| id == gix::ObjectId::empty_blob(id.kind()))
}

/// True if a blob id occurs in more than one deletion or more than one addition.
fn ambiguous(lines: &[String]) -> bool {
    let mut dels = std::collections::HashMap::<&str, usize>::new();
    let mut adds = std::collections::HashMap::<&str, usize>::new();
    for l in lines {
        let f: Vec<&str> = l.splitn(5, ' ').collect();
        if f.len() < 5 || f[2..4].iter().any(|id| is_empty_blob(id)) {
            continue;
        }
        if f[1] == "000000" {
            *dels.entry(f[2]).or_default() += 1;
        } else if f[0] == "000000" {
            *adds.entry(f[3]).or_default() += 1;
        }
    }
    dels.values().any(|n| *n > 1) || adds.values().any(|n| *n > 1)
}

pub fn git_answer(run: &dyn Fn(&[&str]) -> Option<(bool, String, String)>, q: &Query<'_>) -> Option<Ans> {
    let w = &q.words;
    let (mode, a, b) = (*w.first()?, *w.get(1)?, *w.get(2)?);
    let raw = |flags: &[&str]| -> Option<Ans> {
        let args: Vec<&str> = ["diff-tree", "-r", "-z", "--no-abbrev"].iter().chain(flags).chain(&[a, b]).copied().collect();
        let (ok, out, err) = run(&args)?;
        if !ok {
            return Some(Err(err.trim().to_string()));
        }
        parse_raw(&out).map(Ok)
    };
    let plain = raw(&["--no-renames"])?;
    if mode != "exact" {
        return Some(plain);
    }
    if plain.as_ref().is_ok_and(|v| ambiguous(v)) {
        return Some(Ok(vec![AMBIGUOUS.into()]));
    }
    raw(&["-M100%", "--no-rename-empty"])
}

/// Signature area: the most specific feature among the lines only one side has.
pub fn area(q: &Query<'_>, gix: &Ans, git: &Ans) -> String {
    let mode = q.words.first().copied().unwrap_or("nore");
    let (Ok(a), Ok(b)) = (gix, git) else { return mode.to_string() };
    let diff: Vec<&String> = a.iter().filter(|l| !b.contains(l)).chain(b.iter().filter(|l| !a.contains(l))).collect();
    let has = |p: &dyn Fn(&str) -> bool| diff.iter().any(|l| p(l));
    let modes = |l: &str| -> (String, String) {
        let l = l.strip_prefix("R ").or_else(|| l.strip_prefix("C ")).unwrap_or(l);
        let mut f = l.split(' ');
        (f.next().unwrap_or("").to_string(), f.next().unwrap_or("").to_string())
    };
    let feature = if has(&|l| l.starts_with("R ") || l.starts_with("C ")) {
        "rename"
    } else if has(&|l| modes(l).0 == "160000" || modes(l).1 == "160000") {
        "gitlink"
    } else if has(&|l| modes(l).0 == "120000" || modes(l).1 == "120000") {
        "symlink"
    } else if has(&|l| {
        let (o, n) = modes(l);
        o != "000000" && n != "000000" && o != n
    }) {
        "mode"
    } else if has(&|l| {
        let (o, n) = modes(l);
        o == "000000" || n == "000000"
    }) {
        "add_delete"
    } else {
        "modify"
    };
    format!("{mode}_{feature}")
}
