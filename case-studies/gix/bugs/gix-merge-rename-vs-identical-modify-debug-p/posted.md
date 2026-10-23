# gix-merge: debug assertion panic when one side renames and modifies a file and the other side makes the same modification

Posted as https://github.com/GitoxideLabs/gitoxide/issues/3091 on 2026-10-23. The text below is the issue as posted.

---

### Current behavior 😯

Branch `main` renames `a` to `c` and changes its content. Branch `side` makes the same content change to `a`, without renaming it.
`Repository::merge_commits()` panics in builds with debug assertions:

```
thread 'main' panicked at gix-merge/src/tree/utils.rs:138:9:
assertion `left != right` failed: BUG: we must think anything has to be merged if the modes and the ids are the same
  left: EntryMode(0o100644)
 right: EntryMode(0o100644)
```

It also panics with the sides swapped (`side` renames, `main` modifies in place).
A release build without debug assertions does not panic, and it writes the same tree as git.

### Expected behavior 🤔

No panic. A clean merge where `c` has the content that both sides agree on.
The assertion message itself calls reaching it a bug.
The tree merge docs say "Git's `merge-ort` is the behavioral reference" (`gix-merge/src/tree/function/resolve.rs:70`).

### Git behavior

```
git merge-tree --write-tree main side; echo $?
# git 2.56.0: prints a tree, exit status 0 (clean)
git ls-tree <that tree>
# git 2.56.0: 100644 blob b414108e81e5091fe0974a1858b4d0d22b107f70	c      (seq 6)
```

```
repo.merge_commits(main, side, Default::default(), repo.tree_merge_options()?.into())
# gix (debug build): panics at gix-merge/src/tree/utils.rs:138, "BUG: we must think anything has to be merged if the modes and the ids are the same"
# gix (release build): clean, tree has 100644 b414108e81e5091fe0974a1858b4d0d22b107f70 c
```

### Steps to reproduce 🕹

```sh
git init -q -b main repo && cd repo
seq 5 > a && git add a && git commit -q -m base
git checkout -q -b side
seq 6 > a && git commit -q -am theirs                # change a in place
git checkout -q main
git mv a c && seq 6 > c && git commit -q -am ours    # rename a -> c, same change
git merge-tree --write-tree main side                # clean, c = seq 6
```

Then, in that repository, with a debug build of gix:

```rust
let repo = gix::open(".")?;
repo.merge_commits(repo.rev_parse_single("main")?, repo.rev_parse_single("side")?,
                   Default::default(), repo.tree_merge_options()?.into())?;   // panics
```

### Notes

- Seen on gitoxide `main` at e87b6eefb (2026-10-22, debug build, macOS aarch64) and earlier on 59ba38b89 (2026-10-21, Linux), compared with git 2.56.0.
- Likely cause: in `gix-merge/src/tree/function/resolve.rs`, the arm that produces `OursModifiedTheirsRenamedAndChangedThenRename` skips the blob merge only when the rename kept the content (`renamed_without_change`, line 397). It does not skip it when both sides end up with the same id. So `perform_blob_merge()` gets equal ids and equal modes and hits the `debug_assert_ne!` at `gix-merge/src/tree/utils.rs:138`. Other arms already short-circuit on `our_id == their_id` (lines 1564 and 1870).
- Related but different: #2968 (identical add/add reported as `Unknown`, fixed in #2969).

This issue was drafted and submitted by an AI assistant (Claude) on behalf of @alpaylan, at their request, from an automated differential test (gix vs git 2.56).
