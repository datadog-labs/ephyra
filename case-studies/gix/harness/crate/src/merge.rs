// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The `merge` oracle: gix commit merges against `git merge-tree --write-tree` on the same random
//! repositories. Queries are `#merge <ours> <theirs>` (commit ids).
//!
//! Contract: gix-merge/src/tree/function/resolve.rs:68-70 "Git's `merge-ort` is the behavioral
//! reference"; the upstream baseline (gix-merge/tests/merge/tree/mod.rs `run_baseline`) compares
//! merged tree ids with `git merge-tree --write-tree`.
//!
//! Compared: the verdict (git exit 0 = clean, 1 = conflicted; gix
//! `has_unresolved_conflicts(TreatAsUnresolved::git())`, the preset the baseline uses "like in
//! git"), and the merged tree id when clean. A conflicted tree is compared as a listing of
//! `<mode> <id> <path>` where a blob with conflict markers counts as `MARKERS`: where the
//! markers go inside a file is a documented deviation (text/function.rs:116 "DEVIATION ... very
//! different from what Git does"), but which paths, modes and clean blobs a conflicted merge
//! leaves is not. gix runs with object memory, so the repository is not changed for the other
//! oracles.

use crate::graph::{Ans, Query};

fn oid(s: &str) -> Result<gix::ObjectId, String> {
    gix::ObjectId::from_hex(s.as_bytes()).map_err(|e| e.to_string())
}

pub fn gix_answer(repo: &gix::Repository, q: &Query<'_>) -> Ans {
    let w = &q.words;
    let (ours, theirs) = (oid(w.first().ok_or("args")?)?, oid(w.get(1).ok_or("args")?)?);
    let repo = repo.clone().with_object_memory();
    let options = repo.tree_merge_options().map_err(|e| e.to_string())?;
    // git merge-tree labels conflict markers with the commit arguments as given.
    let labels = gix::merge::blob::builtin_driver::text::Labels { ancestor: None, current: Some(w[0].into()), other: Some(w[1].into()) };
    let mut out = repo.merge_commits(ours, theirs, labels, options.into()).map_err(|e| e.to_string())?;
    let conflicted = out.tree_merge.has_unresolved_conflicts(gix::merge::tree::TreatAsUnresolved::git());
    let id = out.tree_merge.tree.write().map_err(|e| e.to_string())?;
    if !conflicted {
        return Ok(vec![format!("clean {id}")]);
    }
    let mut rec = gix::traverse::tree::Recorder::default();
    repo.find_tree(id).map_err(|e| e.to_string())?.traverse().breadthfirst(&mut rec).map_err(|e| e.to_string())?;
    let mut v = vec!["conflict".to_string()];
    for e in rec.records {
        if e.mode.is_tree() {
            continue;
        }
        let markers = e.mode.is_blob() && repo.find_object(e.oid).is_ok_and(|o| has_markers(&o.data));
        v.push(format!("{:06o} {} {}", e.mode.kind() as u16, if markers { "MARKERS".to_string() } else { e.oid.to_string() }, e.filepath));
    }
    v[1..].sort();
    Ok(v)
}

/// git's answer when its merge inferred a directory rename from file renames. gix tracks renamed
/// directories only by identity: crate-status.md:374-376 "directory tracking / [x] by identity /
/// [ ] by similarity", and the rename tracker differs from git by design (gix-diff/src/rewrites/
/// tracker.rs:1-7). Such merges have no reference; they are counted and skipped.
pub const DIR_RENAME: &str = "?directory-rename";

/// merge-ort's messages for paths moved (or suggested to move) because of an inferred directory rename.
fn mentions_dir_rename(out: &str) -> bool {
    ["inside a directory that was renamed", "CONFLICT (implicit dir rename)", "CONFLICT (directory rename split)"]
        .iter()
        .any(|m| out.contains(m))
}

/// A line of at least 7 `<` followed by a space. merge-ort lengthens markers for nested
/// conflicts (its message: "this may result in nested conflict markers"), e.g. `<<<<<<<< ours:path`
/// when a renamed file with content conflicts collides with another path.
fn has_markers(data: &[u8]) -> bool {
    data.split(|&b| b == b'\n').any(|l| {
        let n = l.iter().take_while(|&&b| b == b'<').count();
        n >= 7 && l.get(n) == Some(&b' ')
    })
}

pub fn git_answer(run: &dyn Fn(&[&str]) -> Option<(bool, String, String)>, q: &Query<'_>) -> Option<Ans> {
    let w = &q.words;
    let (ours, theirs) = (*w.first()?, *w.get(1)?);
    let (ok, out, err) = run(&["merge-tree", "--write-tree", ours, theirs])?;
    if mentions_dir_rename(&out) {
        return Some(Ok(vec![DIR_RENAME.into()]));
    }
    // Warnings (e.g. "unable to access 'd/.gitattributes': Too many levels of symbolic links" when
    // the worktree has a symlink loop) are not part of the answer.
    let err: String = err.lines().filter(|l| !l.starts_with("warning: ")).collect::<Vec<_>>().join("\n");
    let first = out.lines().next().unwrap_or("").trim().to_string();
    if ok {
        return Some(Ok(vec![format!("clean {first}")]));
    }
    // Exit 1 with a tree id: conflicted. Anything else is an error.
    if err.trim().is_empty() && first.len() >= 40 && first.bytes().all(|b| b.is_ascii_hexdigit()) {
        let (ok, ls, _) = run(&["ls-tree", "-r", "-z", &first])?;
        if !ok {
            return None;
        }
        let mut v = vec!["conflict".to_string()];
        for rec in ls.split('\0').filter(|r| !r.is_empty()) {
            let (meta, path) = rec.split_once('\t')?;
            let f: Vec<&str> = meta.split(' ').collect();
            let (mode, kind, id) = (*f.first()?, *f.get(1)?, *f.get(2)?);
            let markers = kind == "blob" && run(&["cat-file", "blob", id]).is_some_and(|(_, c, _)| has_markers(c.as_bytes()));
            v.push(format!("{mode} {} {path}", if markers { "MARKERS" } else { id }));
        }
        v[1..].sort();
        return Some(Ok(v));
    }
    Some(Err(err.trim().to_string()))
}

type Run<'a> = &'a dyn Fn(&[&str]) -> Option<(bool, String, String)>;

/// `path -> (mode, id)` of the blobs (and links) in a commit's tree, via git.
fn ls_tree(run: Run<'_>, rev: &str) -> Option<std::collections::HashMap<String, (String, String)>> {
    let (ok, ls, _) = run(&["ls-tree", "-r", "-z", rev])?;
    ok.then(|| {
        ls.split('\0')
            .filter_map(|rec| {
                let (meta, path) = rec.split_once('\t')?;
                let f: Vec<&str> = meta.split(' ').collect();
                Some((path.to_string(), (f.first()?.to_string(), f.get(2)?.to_string())))
            })
            .collect()
    })
}

fn merge_bases(run: Run<'_>, ours: &str, theirs: &str) -> Option<Vec<String>> {
    let (_, out, _) = run(&["merge-base", "--all", ours, theirs])?;
    Some(out.lines().map(str::to_string).collect())
}

/// git's answer is not a reference when one side renames `P` to `Q` unchanged (exact rename) and
/// the other side modifies `P` and adds `Q` with `P`'s original content. merge-ort then resolves
/// `Q` without a conflict but differently per direction (black box, git 2.56: base `a`="1", A:
/// `git mv a b`, B: a=`seq 1 5`, b="1"; `merge-tree A B` gives b=`seq 1 5`, `merge-tree B A` gives
/// b="1" and silently drops B's edit of `a`). gix reports the rename/add conflict that merge-ort
/// reports when the addition has any other content (upstream baseline `rename-add`,
/// gix-merge/tests/fixtures/tree-baseline.sh).
pub fn git_rename_add_of_source(run: Run<'_>, q: &Query<'_>) -> bool {
    let w = &q.words;
    let (Some(ours), Some(theirs)) = (w.first(), w.get(1)) else { return false };
    let (Some(o), Some(t), Some(bases)) = (ls_tree(run, ours), ls_tree(run, theirs), merge_bases(run, ours, theirs)) else {
        return false;
    };
    bases.iter().filter_map(|b| ls_tree(run, b)).any(|base| {
        let pattern = |renamer: &std::collections::HashMap<String, (String, String)>, adder: &std::collections::HashMap<String, (String, String)>| {
            base.iter().any(|(p, (_, x))| {
                !renamer.contains_key(p)
                    && adder.get(p).is_some_and(|(_, id)| id != x)
                    && renamer.iter().any(|(q, (_, id))| {
                        id == x && !base.contains_key(q) && adder.get(q).is_some_and(|(_, aid)| aid == x)
                    })
            })
        };
        pattern(&o, &t) || pattern(&t, &o)
    })
}

/// git's answer is not a reference when one side's exact renames are ambiguous: relative to a merge
/// base it deletes a blob `X` and adds more than one new path with `X` (or deletes several paths
/// with `X` and adds one). Which pairing is "the" rename is a heuristic, and gix's rename tracker
/// differs from git's by design (gix-diff/src/rewrites/tracker.rs:1-6: "the algorithm implemented
/// here is in many ways different from what `git` does ... it's less sophisticated than `git`").
/// Black box, git 2.56: base d/c=X; ours `git mv d/c d.mv` plus new d/e/f=X; theirs edits d/c.
/// git applies theirs' edit to d.mv, gix to d/e/f; both are valid merges. The treediff oracle
/// skips the same ambiguity (treediff.rs `ambiguous`).
pub fn ambiguous_exact_renames(run: Run<'_>, q: &Query<'_>) -> bool {
    let w = &q.words;
    let (Some(ours), Some(theirs)) = (w.first(), w.get(1)) else { return false };
    let (Some(o), Some(t), Some(bases)) = (ls_tree(run, ours), ls_tree(run, theirs), merge_bases(run, ours, theirs)) else {
        return false;
    };
    bases.iter().filter_map(|b| ls_tree(run, b)).any(|base| {
        [&o, &t].iter().any(|side| {
            let mut dels = std::collections::HashMap::<&str, usize>::new();
            let mut adds = std::collections::HashMap::<&str, usize>::new();
            for (p, (_, id)) in &base {
                if !side.contains_key(p) {
                    *dels.entry(id.as_str()).or_default() += 1;
                }
            }
            for (p, (_, id)) in side.iter() {
                if !base.contains_key(p) {
                    *adds.entry(id.as_str()).or_default() += 1;
                }
            }
            dels.iter().any(|(id, n)| adds.get(id).is_some_and(|m| n + m > 2))
        })
    })
}

/// gix's answer with documented add/add mode choices replaced by git's. For an add/add conflict
/// (path in no merge base) of a non-executable and an executable blob, git keeps our mode while
/// gix "consistently use[s] +x" (gix-merge/tests/fixtures/tree-baseline.sh, case
/// `added-file-changed-content-and-mode`: "We improve on executable bit handling"; resolve.rs
/// `merge_modes`: "for two blob modes, prefer executable"). git's add/add also covers rename/add,
/// where the renamed side's merged mode is +x (e.g. A edits `a` and sets +x, B renames `a` to `f`,
/// A adds a different `f`): git's conflict stages for the path are then 2 and 3, one 100644 and
/// one 100755, with no stage 1. `None` if nothing was replaced.
pub fn mask_add_add_exec(run: Run<'_>, q: &Query<'_>, gix: &Ans, git: &Ans) -> Option<Ans> {
    let (Ok(a), Ok(b)) = (gix, git) else { return None };
    if a == b || a.first().map(String::as_str) != Some("conflict") || b.first().map(String::as_str) != Some("conflict") {
        return None;
    }
    let w = &q.words;
    let (ours, theirs) = (w.first()?, w.get(1)?);
    let (o, t) = (ls_tree(run, ours)?, ls_tree(run, theirs)?);
    let bases: Vec<_> = merge_bases(run, ours, theirs)?.iter().filter_map(|b| ls_tree(run, b)).collect();
    // git's conflicted stages: path -> [(mode, stage)].
    let mut stages = std::collections::HashMap::<String, Vec<(String, String)>>::new();
    if let Some((_, out, _)) = run(&["merge-tree", "--write-tree", ours, theirs]) {
        for l in out.lines().skip(1).take_while(|l| !l.is_empty()) {
            let Some((meta, path)) = l.split_once('\t') else { continue };
            let f: Vec<&str> = meta.split(' ').collect();
            if let (Some(m), Some(st)) = (f.first(), f.get(2)) {
                stages.entry(path.to_string()).or_default().push((m.to_string(), st.to_string()));
            }
        }
    }
    let git_add_add_exec = |path: &str| {
        stages.get(path).is_some_and(|v| {
            let mut v = v.clone();
            v.sort();
            v == [("100644".to_string(), "2".to_string()), ("100755".to_string(), "3".to_string())]
                || v == [("100644".to_string(), "3".to_string()), ("100755".to_string(), "2".to_string())]
        })
    };
    let mut masked = false;
    let v: Vec<String> = a
        .iter()
        .map(|l| {
            let Some(path) = l.strip_prefix("100755 MARKERS ") else { return l.clone() };
            let git_line = format!("100644 MARKERS {path}");
            let add_add = !bases.iter().any(|base| base.contains_key(path))
                && o.get(path).is_some_and(|(m, _)| m == "100644")
                && t.get(path).is_some_and(|(m, _)| m == "100755");
            let add_add = add_add || git_add_add_exec(path);
            if add_add && b.contains(&git_line) && !b.contains(l) {
                masked = true;
                git_line
            } else {
                l.clone()
            }
        })
        .collect();
    masked.then(|| {
        let mut v = v;
        v[1..].sort();
        Ok(v)
    })
}
