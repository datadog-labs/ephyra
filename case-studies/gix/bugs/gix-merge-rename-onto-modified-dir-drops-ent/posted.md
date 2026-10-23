# gix-merge: tree merge loses a file renamed onto a directory that the other side modified

Posted as https://github.com/GitoxideLabs/gitoxide/issues/3109 on 2026-10-23. The text below is the issue as posted.

---

### Current behavior 😯

The base has a directory `d` (with `d/f`) and a file `a`. One side modifies `d/f`. The other side deletes `d` and renames `a` to `d`.

`gix merge commit` reports a conflict, but one side's content is missing from the merged tree.

- When merging `ours theirs`, the renamed file (content of `a`) is neither at `d` nor at `d~theirs`.
- When merging `theirs ours`, the renamed file is at `d`, and the modified `d/f` is gone.

```
gix merge commit ours theirs   # then: git ls-tree -r <tree>
# gix: 100644 blob 0cfbf08...  d/f
gix merge commit theirs ours
# gix: 100644 blob 7898192...  d
```

### Expected behavior 🤔

The merged tree should keep both the modified `d/f` and the renamed file, the same way `git merge-tree` does. Merges must not silently drop one side's content.

`gix-merge/src/tree/function/resolve.rs` says "Git's `merge-ort` is the behavioral reference". gix already gets this right when the other side adds a new file `d` instead of renaming `a` to `d`. In that case it keeps `d/f` and writes the new file to `d~theirs`, exactly like git.

### Git behavior

git 2.55.0 and 2.56.0 give the same tree in both directions:

```
git merge-tree --write-tree ours theirs   # then: git ls-tree -r <tree>
# git: 100644 blob 0cfbf08...  d/f
# git: 100644 blob 7898192...  d~theirs
# CONFLICT (modify/delete): d/f deleted in theirs and modified in ours.  Version ours of d/f left in tree.
# CONFLICT (file/directory): directory in the way of d from theirs; moving it to d~theirs instead.
git merge-tree --write-tree theirs ours
# git: 100644 blob 0cfbf08...  d/f
# git: 100644 blob 7898192...  d~theirs
```

### Steps to reproduce 🕹

```sh
git init -q -b main repo && cd repo
mkdir d && echo 1 > d/f && echo a > a && git add . && git commit -q -m base
git checkout -q -b ours
echo 2 > d/f && git commit -q -am "modify d/f"
git checkout -q -b theirs main
git rm -q -r d && git mv a d && git commit -q -m "rm -r d; mv a d"

git ls-tree -r $(git merge-tree --write-tree ours theirs | head -1)
# git: d/f and d~theirs
git ls-tree -r $(gix merge commit ours theirs 2>/dev/null | cut -d' ' -f1)
# gix: only d/f (the renamed file is lost)
git ls-tree -r $(gix merge commit theirs ours 2>/dev/null | cut -d' ' -f1)
# gix: only d (the modified d/f is lost)
```

The same happens when `a` is a symlink instead of a regular file.

### Notes

- Related: #3106 (a false directory/file conflict) is also a directory-vs-file case in the tree merge. They are separate defects: a fix for either one does not fix the other.
- Seen on gitoxide `main` at e87b6eefb (2026-10-22, macOS aarch64; the current main f984c56f0 only changes a test) and earlier on 59ba38b89 (2026-10-21, Linux), compared with git 2.56.0.
  options.
- Where in the code: `gix-merge/src/tree/function/resolve.rs` around the `matching_change(...)` call in `tree()`,
  together with `change::matching` in `gix-merge/src/tree/function/change.rs`. The rename's destination `d` is only
  checked against our own rewrites. So the rename gets no conflict candidate, and it is applied over our directory `d`. The modification of `d/f` is then applied too, and one entry replaces the other.
- This probably came in with 43cf77fdb ("change!: Refactor tree-merge change matching and resolution"). That commit
  was first released in gix-merge v0.20.0. Older releases were not checked.
- Related, but a different case: #3024 (also a file/directory merge losing content, but without a rename).

This issue was drafted and submitted by an AI assistant (Claude) on behalf of @alpaylan, at their request, from an automated differential test (gix vs git 2.56).
