# `status()` / `gix_diff::index()` miss a staged deletion when the index entry is intent-to-add

Posted as https://github.com/GitoxideLabs/gitoxide/issues/3115 on 2026-10-23. The text below is the issue as posted.

---

### Current behavior 😯

A file is tracked in `HEAD`. Its index entry is replaced by an intent-to-add entry (`git rm --cached a; git add -N a`).
Git sees this as a staged deletion. `Repository::status()` reports no HEAD-vs-index change for `a`.

```sh
gix status (Item::TreeIndex changes only)
# gix: (nothing)
```

If the entry also has the skip-worktree bit, gix reports nothing at all for `a`.

### Expected behavior 🤔

A `Deletion` of `a` in the tree-vs-index part of the status, like Git.
`Repository::status()` is documented as "most similar to `git status --ignored=no`" (`gix/src/status/mod.rs:71-73`).
git-add(1) says of `-N`: "An entry for the path is placed in the index with no content."

```sh
gix status (Item::TreeIndex changes only)
# git: D a
# gix: D a
```

### Git behavior

```sh
git status --short
# git: DA a
git diff --cached --name-status
# git: D	a
git write-tree
# git: 4b825dc642cb6eb9a060e54bf8d69288fbee4904   (the empty tree: `a` is dropped)
git commit -m x
# git: the commit deletes `a` (1 file changed, 1 deletion)
```

So committing this index deletes the file, but gix's status does not show that.

### Steps to reproduce 🕹

```sh
git init -q repo && cd repo
echo 1 > a
git add a && git commit -q -m init
git rm -q --cached a
git add -N a
git diff --cached --name-status   # D	a
```

Then, with `gix` (features `status`, `sha1`):

```rust
gix::open(".")?.status(gix::progress::Discard)?.into_iter(None)?
    .filter_map(|i| match i.ok()? { gix::status::Item::TreeIndex(c) => Some(c), _ => None })
    .count() // 0, expected 1 (Deletion of `a`)
```

### Notes

- Seen on gitoxide `main` at e87b6eefb (2026-10-22, macOS aarch64; the current main f984c56f0 only changes a test) and earlier on 59ba38b89 (2026-10-21, Linux), compared with git 2.56.0.
- Cause: in `gix-diff/src/index/function.rs`, the `Ordering::Equal` branch of `diff()` skips both entries when the
  right-hand entry is intent-to-add (`ignore_unmerged_and_intent_to_add(rhs)`), so the left-hand entry is dropped
  without a `Deletion`. An intent-to-add entry for a path that is new on the right is correctly ignored.
  `gix/src/status/tree_index.rs` uses `gix_diff::index()`, so status inherits it.
- The code dates from early 2025 (`gix_diff::index()` was added then); first released in gix v0.70.0.
- No related upstream issue found. #1770 is a different missing-staged-change case (unborn HEAD).

This issue was drafted and submitted by an AI assistant (Claude) on behalf of @alpaylan, at their request, from an automated differential test (gix vs git 2.56).
