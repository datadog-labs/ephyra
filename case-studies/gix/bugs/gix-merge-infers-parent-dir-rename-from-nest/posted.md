# gix-merge: moving `d/e` to `y/z` makes the tree merge treat `d` as renamed to `y`

Posted as https://github.com/GitoxideLabs/gitoxide/issues/3108 on 2026-10-23. The text below is the issue as posted.

---

### Current behavior 😯

The base has `d/e/f`. One side moves the subdirectory `d/e` to `y/z`, so `d` is gone on that side. The other side adds a new file `d/c`.

`gix merge commit` moves the new file to `y/c`. It acts as if the whole directory `d` was renamed to `y`. It exits 0 and prints "1 possibly resolved conflicts". The result is the same in both merge directions.

```
gix merge commit ours theirs     # then: git ls-tree -r --name-only <tree>
# gix: y/c
# gix: y/z/f
```

### Expected behavior 🤔

The new file should stay at `d/c`, as it does with `git merge-tree`. `gix-merge/src/tree/function/resolve.rs` says "Git's `merge-ort` is the behavioral reference".

Only `d/e` was renamed (to `y/z`). `d` itself was not renamed to `y`, because the path below it changed (`e` became `z`). When the subdirectory keeps its name (`d/e` to `y/e`), git does move the addition to `y/c`, and gix gives the same result there.

### Git behavior

git 2.56.0 merges cleanly and keeps `d/c`, in both directions:

```
git merge-tree --write-tree ours theirs   # then: git ls-tree -r --name-only <tree>
# git: d/c
# git: y/z/f
git merge-tree --write-tree theirs ours
# git: d/c
# git: y/z/f
```

Control case, where the subdirectory keeps its name (`git mv d/e y/e` instead of `git mv d/e y/z`):

```
git merge-tree --write-tree ours theirs
# git: y/c
# git: y/e/f
gix merge commit ours theirs
# gix: y/c
# gix: y/e/f
```

### Steps to reproduce 🕹

```sh
git init -q -b main repo && cd repo
mkdir -p d/e && echo 1 > d/e/f && git add . && git commit -q -m base
git checkout -q -b ours
mkdir y && git mv d/e y/z && git commit -q -m "mv d/e y/z"
git checkout -q -b theirs main
echo x > d/c && git add d/c && git commit -q -m "add d/c"

git ls-tree -r --name-only $(git merge-tree --write-tree ours theirs | head -1)
# git: d/c  y/z/f
git ls-tree -r --name-only $(gix merge commit ours theirs 2>/dev/null | cut -d' ' -f1)
# gix: y/c  y/z/f
```

### Notes

- Seen on gitoxide `main` at e87b6eefb (2026-10-22, macOS aarch64; the current main f984c56f0 only changes a test) and earlier on 59ba38b89 (2026-10-21, Linux), compared with git 2.56.0.
- Where in the code: `Tracker::match_renamed_directories` in `gix-diff/src/rewrites/tracker.rs`. It emits a rename of the deleted top-level directory `d` to the added directory `y` whenever any exact child rename links them ("one rewrite inside the directory is enough"), no matter what the child's path below each directory is. The tree merge then follows that directory rename.
- This probably came in with ba7b811bd ("feat: support rename tracking across changed directories"), first released in gix-v0.68.0. Older releases were not checked.
- The merge baseline already records this as a deviation: `gitlink-vs-renamed-symlink-directory-with-siblings` in `gix-merge/tests/fixtures/tree-baseline.sh` says "gix relocates A's addition through the detected directory rename, while Git keeps it at its original path; FIXME".
- #2687 (order-independent rename tracking) is already on main; the bug still reproduces with it.

This issue was drafted and submitted by an AI assistant (Claude) on behalf of @alpaylan, at their request, from an automated differential test (gix vs git 2.56).
