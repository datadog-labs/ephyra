# `status()` lists a file that replaced a submodule (gitlink) as untracked, in addition to the type change

Posted as https://github.com/GitoxideLabs/gitoxide/issues/3113 on 2026-10-23. The text below is the issue as posted.

---

### Current behavior 😯

The index has a gitlink (mode `160000`) at `f`. The worktree has a regular file `f` instead.
`Repository::status()` returns two index-worktree items for `f`: a type change, and an untracked entry from the directory walk.

```
gix status (index/worktree items, via Item::summary())
# gix: Added f
# gix: TypeChange f
```

### Expected behavior 🤔

Only the type change. `f` has an index entry, so it is tracked and should not also appear as untracked.
The docs of `Repository::status()` say it is "most similar to `git status --ignored=no`".

```
gix status (index/worktree items, via Item::summary())
# gix (expected): TypeChange f
```

### Git behavior

```
git status --porcelain=v2
# git: 1 .T S... 160000 160000 100644 <id> <id> f
git ls-files -o
# git: (no output)
```

Git reports the type change only and does not list `f` as untracked.

### Steps to reproduce 🕹

```sh
git init -b main repo && cd repo
git commit --allow-empty -m init
git update-index --add --cacheinfo "160000,$(git rev-parse HEAD),f"
git commit -m sub
echo x > f
```

Then, on that repository:

```rust
for item in gix::open(".")?.status(gix::progress::Discard)?.into_index_worktree_iter(None)? {
    let item = item?;
    if let Some(s) = item.summary() { println!("{s:?} {}", item.rela_path()); }
}
// prints "Added f" and "TypeChange f"
```

A symlink at `f` gives the same extra untracked entry in gix. A submodule added with `git submodule add` and then replaced by a regular file behaves the same way too.

### Notes

- Seen on gitoxide `main` at e87b6eefb (2026-10-22, macOS aarch64; the current main f984c56f0 only changes a test) and earlier on 59ba38b89 (2026-10-21, Linux), compared with git 2.56.0.
- Checked on gitoxide `main` at 59ba38b89 (2026-10-21) with git 2.55.0.
- Location: `gix-dir/src/walk/classify.rs`, `path()`. It marks the entry as tracked only when `index_kind.is_dir() == kind.is_dir()`. `Kind::Repository.is_dir()` is true, so a gitlink in the index and a file on disk never match, and the path is classified as untracked. This comparison has been there since the first dirwalk implementation (3252cfd57, gix-dir 0.1.0).
- The gix-dir test `submodule_replaced_with_symlink_without_assume_unchanged_is_untracked` (02a815c79, added with #2437) asserts the current behaviour for the symlink case. For that case, `git status` fails with `expected submodule path 'f' not to be a symbolic link`, and `git ls-files -o` prints nothing.
- Related, but different: #2435 / #2437 (assume-unchanged submodule replaced by a symlink), #3026 (uninitialized submodule directories), #2490 / #2673 (empty directories).

This issue was drafted and submitted by an AI assistant (Claude) on behalf of @alpaylan, at their request, from an automated differential test (gix vs git 2.56).
