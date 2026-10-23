# Tree merge reports a false directory/file conflict when one side replaces `d/` with a file and the other only deletes part of `d/`

Posted as https://github.com/GitoxideLabs/gitoxide/issues/3106 on 2026-10-23. The text below is the issue as posted.

---

### Current behavior 😯

The base has `d/c` and `d/f`. Our branch replaces the directory `d` with a file `d`. Their branch only deletes `d/f`.
`gix merge commit` reports a tree conflict and moves our new file to `d~main`. The merged tree has no `d`.

```
$ gix merge commit main theirs
# gix: 8622ebc5… (wrote 1 trees)
# gix: 1 possibly resolved conflicts
# gix: Error: Tree conflicted
$ git ls-tree 8622ebc5
# gix: 100644 blob 587be6b4…  d~main
```

Merging in the other direction (`gix merge commit theirs main`) gives the same result: a conflict, and the file ends up at `d~main`.

### Expected behavior 🤔

A clean merge with the file at `d`, like `git merge-tree`. Nothing below `d` is left to collide with the new file.
Our side deletes all of `d/`. Their side deletes `d/f`, which our side deletes too, and does not touch `d/c`.
`gix-merge/src/tree/function/resolve.rs` says: "Git's `merge-ort` is the behavioral reference".

### Git behavior

```
$ git merge-tree --write-tree main theirs; echo $?
# git 2.56: c79d803e4016190987f675c0487a31206aa782f8
# git 2.56: 0
$ git ls-tree c79d803e
# git 2.56: 100644 blob 587be6b4…  d
$ gix merge commit main theirs    # then: git ls-tree <tree>
# gix: 100644 blob 587be6b4…  d~main     (and "Error: Tree conflicted")
```

### Steps to reproduce 🕹

```sh
git init -q -b main repo && cd repo
mkdir d && echo c > d/c && echo f > d/f && git add d && git commit -q -m base
git branch theirs
git rm -q -r d && echo x > d && git add d && git commit -q -m "d becomes a file"
git checkout -q theirs && git rm -q d/f && git commit -q -m "delete d/f" && git checkout -q main

git ls-tree $(git merge-tree --write-tree main theirs | head -1)   # d
git ls-tree $(gix merge commit main theirs | cut -d' ' -f1)        # d~main, plus "Tree conflicted"
```

The same thing through the library: `repo.merge_commits(main, theirs, labels, repo.tree_merge_options()?.into())`.

### Notes

- Seen on gitoxide `main` at e87b6eefb (2026-10-22, macOS aarch64; the current main f984c56f0 only changes a test) and earlier on 59ba38b89 (2026-10-21, Linux), compared with git 2.56.0.
- Where in the code: the "structural-only overlap" branch in `gix-merge/src/tree/function/resolve.rs`
  (`let Some(ours_idx) = ours_idx else { … }`, around line 252). The partly deleting side's path index still has
  a node for the deletion that both sides share, `d/f`. This happens whether that deletion was already applied
  or is still pending. So the new non-tree `d` is seen as a `TreeToNonTree` overlap with no change at `d` itself.
  The code then takes the "ours modifies `dir/file`, theirs replaces `dir` with a blob" path, which keeps the
  directory and renames the blob.
- That branch came in with 43cf77fdb ("Refactor tree-merge change matching and resolution"), first released in
  gix-merge v0.20.0, so released versions from v0.20.0 on are likely affected. Older releases were not checked.
- Related but different: #3024 (file replaced by a directory made of renames; the directory was dropped), #2826
  (additions below jointly deleted files). Both are merged and on the commit above.

This issue was drafted and submitted by an AI assistant (Claude) on behalf of @alpaylan, at their request, from an automated differential test (gix vs git 2.56).
