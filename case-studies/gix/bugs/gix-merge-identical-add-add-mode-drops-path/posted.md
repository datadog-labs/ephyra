# gix-merge: same blob on both sides with different modes falls through to `Unknown` (path dropped, or ancestor kept)

Posted as https://github.com/GitoxideLabs/gitoxide/issues/3097 on 2026-10-23. The text below is the issue as posted.

---

Two cases where both sides end up with the same blob id but different modes. Both pass the `our_id != their_id` checks and land in the `_unknown` fallback: one in the add/add arm, one in the modify/modify arm. Each has its own reproduction below.

### Current behavior 😯

**1. gix-merge: same file added on both sides, executable on one side only, is missing from the merged tree**

Both sides add a new file `b` with the same content. Only one side makes it executable.
`Repository::merge_commits()` (and `merge_trees()`) writes a merged tree without `b`. It records one conflict
with resolution `Err(Unknown)`.

Both sides added this file, yet it is gone from the result. A caller that commits the tree anyway loses `b`.

**2. gix-merge: same content on both sides plus a one-sided `chmod +x` keeps the ancestor and reports `Unknown`**

Both sides change file `f` to the same new content. Only one side also makes it executable.
`Repository::merge_commits()` (and `merge_trees()`) puts the *ancestor* entry of `f` into the merged tree: old
content, old mode. It also records one conflict with resolution `Err(Unknown)`, so
`has_unresolved_conflicts()` is `true`.

Neither side has the ancestor content any more. A caller that commits the tree anyway reverts both sides' change.

### Expected behavior 🤔

**1. gix-merge: same file added on both sides, executable on one side only, is missing from the merged tree**

`b` is in the merged tree. gix already documents its choice for the closely related rename case in
`gix-merge/tests/fixtures/tree-baseline.sh` (`identical-renames-to-same-destination-with-mode-change`):
"gix resolves the identical content and mode change cleanly, while Git leaves an add/add mode conflict".
`merge_modes()` in `resolve.rs` says: "for two blob modes, prefer executable".
So the expected result is a clean merge with `b` as 100755. At minimum, `b` must not disappear.

**2. gix-merge: same content on both sides plus a one-sided `chmod +x` keeps the ancestor and reports `Unknown`**

A clean merge: `f` gets the new content (which both sides agree on) and mode 100755 (changed on one side only).
No conflict.

The tree merge docs name merge-ort as the yardstick: "Git's `merge-ort` is the behavioral reference"
(`gix-merge/src/tree/function/resolve.rs:70`).

### Git behavior

**1. gix-merge: same file added on both sides, executable on one side only, is missing from the merged tree**

```
git merge-tree --write-tree main side; echo $?
# git 2.56.0: exit 1, CONFLICT (add/add): Merge conflict in b
#             100644 0cfbf08... 2	b
#             100755 0cfbf08... 3	b
git ls-tree <tree written by git>
# git 2.56.0: 100644 blob d00491f...	a
#             100644 blob 0cfbf08...	b
```

```
repo.merge_commits(main, side, Default::default(), repo.tree_merge_options()?.into())?
# gix: tree_merge.conflicts = [Err(Unknown)]
git ls-tree <tree written by gix>
# gix: 100644 blob d00491f...	a
#      (no b)
```

**2. gix-merge: same content on both sides plus a one-sided `chmod +x` keeps the ancestor and reports `Unknown`**

```
git merge-tree --write-tree main side; echo $?
# git 2.56.0: prints a tree, exit status 0 (clean)
git ls-tree <that tree>
# git 2.56.0: 100755 blob 0cfbf08886fca9a91cb753ec8734c84fcbe52c9f	f      ("2", executable)
```

```
repo.merge_commits(main, side, Default::default(), repo.tree_merge_options()?.into())?
# gix: tree_merge.conflicts = [Err(Unknown)], has_unresolved_conflicts(Default::default()) = true
git ls-tree <tree written by gix>
# gix: 100644 blob d00491fd7e5bb6fa28c517a0bb32b8b506539d4d	f      ("1", the ancestor)
```

### Steps to reproduce 🕹

**1. gix-merge: same file added on both sides, executable on one side only, is missing from the merged tree**

```sh
git init -q -b main repo && cd repo
echo 1 > a && git add a && git commit -q -m base
git checkout -q -b side
echo 2 > b && chmod +x b && git add b && git commit -q -m theirs   # adds b, executable
git checkout -q main
echo 2 > b && git add b && git commit -q -m ours                    # adds the same b, not executable
git merge-tree --write-tree main side                               # b kept (add/add mode conflict)
```

Then, in that repository:

```rust
let repo = gix::open(".")?;
let mut out = repo.merge_commits(repo.rev_parse_single("main")?, repo.rev_parse_single("side")?,
                                 Default::default(), repo.tree_merge_options()?.into())?;
// out.tree_merge.conflicts == [Err(Unknown)]; out.tree_merge.tree.write()? has only `a`
```

**2. gix-merge: same content on both sides plus a one-sided `chmod +x` keeps the ancestor and reports `Unknown`**

```sh
git init -q -b main repo && cd repo
echo 1 > f && git add f && git commit -q -m base
git checkout -q -b side
echo 2 > f && chmod +x f && git commit -q -am theirs   # new content + executable
git checkout -q main
echo 2 > f && git commit -q -am ours                    # same new content, mode unchanged
git merge-tree --write-tree main side                   # clean, f is 100755 "2"
```

Then, in that repository:

```rust
let repo = gix::open(".")?;
let mut out = repo.merge_commits(repo.rev_parse_single("main")?, repo.rev_parse_single("side")?,
                                 Default::default(), repo.tree_merge_options()?.into())?;
// out.tree_merge.conflicts == [Err(Unknown)]; out.tree_merge.tree.write()? has `f` as 100644 "1"
```

### Notes

- Seen on gitoxide `main` at e87b6eefb (2026-10-22, macOS aarch64; the current main f984c56f0 only changes a test) and earlier on 59ba38b89 (2026-10-21, Linux), compared with git 2.56.0.
- **1.** git 2.56.0, on Linux. The mirrored case (`main` adds it executable, `side` does not) also drops `b`.
  - Likely cause: in `gix-merge/src/tree/function/resolve.rs`, the add/add arm has the guard
  `our_id != their_id` (line 832). With equal ids and different modes, no other arm matches. The pair falls
  to the `_unknown` fallback (line 1992), which leaves the ancestor in place. The ancestor has no `b`.
  - Related: #2968 (identical add/add with equal modes reported as `Unknown`, fixed in #2969). That fix does not
  cover this case, because the modes differ here.
- **2.** on Linux.
  - Likely cause: in `gix-merge/src/tree/function/resolve.rs`, the modify/modify arm has the guard
  `our_id != their_id` (line 525). When the ids are equal but the modes differ, no other arm matches.
  `same_effect()` (`gix-merge/src/tree/function/change.rs:193`) also compares modes. The pair falls through to the
  `_unknown` fallback (line 1992), which leaves the ancestor in place and records `Conflict::unknown`.
  - Related: #2968 (identical add/add reported as `Unknown`, fixed in #2969). That fix does not cover this case,
  because the modes are not equal here.
  - The same happens in the mirrored case (`main` makes `f` executable, `side` does not).

This issue was drafted and submitted by an AI assistant (Claude) on behalf of @alpaylan, at their request, from an automated differential test (gix vs git 2.56).
