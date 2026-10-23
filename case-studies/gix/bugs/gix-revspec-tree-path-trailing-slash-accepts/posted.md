# `rev_parse` resolves `HEAD:file/` to the blob of `file`; git rejects a trailing slash on a non-directory

Posted as https://github.com/GitoxideLabs/gitoxide/issues/3102 on 2026-10-23. The text below is the issue as posted.

---

### Current behavior 😯

A `<rev>:<path>` spec whose path ends in `/` resolves even when the path names a file. gix returns the blob of the file:

```sh
gix rev parse HEAD:file/
# git: fatal: path 'file/' does not exist in 'HEAD'
# gix: 45b983be36b73c0788dc9cbcb76cbb80fc7bb057   (the blob of `file`)
```

The automated test also hit this with symlink and submodule (gitlink) entries, on an earlier commit (d568e8b78). Paths with empty or `.` components are also accepted and normalised:

```sh
gix rev parse HEAD:dir//f
# git: fatal: path 'dir//f' exists on disk, but not in 'HEAD'
# gix: 587be6b4c3f93f93c489c0111bba5596147a26cb   (the blob of `dir/f`)

gix rev parse HEAD:dir/./f
# git: rev-parse --verify fails
# gix: 587be6b4c3f93f93c489c0111bba5596147a26cb
```

### Expected behavior 🤔

A path ending in `/` names a directory, so `HEAD:file/` should fail to resolve, as it does in git. `HEAD:dir/` should keep resolving to the tree of `dir`, which gix already does. `HEAD:dir//f` and `HEAD:dir/./f` should fail too, matching git.

### Git behavior

git 2.56.0 (and 2.55.0) looks up each path component verbatim:

```sh
git rev-parse --verify HEAD:file/
# git: fatal: path 'file/' does not exist in 'HEAD'
git rev-parse --verify HEAD:dir/
# git: a1dffc7a64c0b2d395484bf452e9aeb1da3a18f2   (tree of dir; gix agrees)
git rev-parse --verify HEAD:dir//f
# git: fatal: path 'dir//f' exists on disk, but not in 'HEAD'
```

### Steps to reproduce 🕹

```sh
git init -q repo && cd repo
mkdir dir && echo hi > file && echo x > dir/f
git add -A && git commit -q -m init
git rev-parse --verify HEAD:file/   # fails
gix rev parse HEAD:file/            # prints the blob id of `file`
```

`gix rev parse` calls `Repository::rev_parse()`, so library callers get the same result.

### Notes

- Seen on gitoxide `main` at e87b6eefb (2026-10-22, macOS aarch64; the current main f984c56f0 only changes a test) and earlier on 59ba38b89 (2026-10-21, Linux), compared with git 2.56.0.
- Where: `gix/src/revision/spec/parse/delegate/navigate.rs`, the `PeelTo::Path` branch calls `Tree::peel_to_entry_by_path()`. That function splits the path with `std::path::Path::components()`, which drops a trailing `/` and skips empty and `.` components.
- `Tree::peel_to_entry_by_path()` is also used in `gix/src/submodule/mod.rs`; that caller was not checked.
- Related but different: #2456 (what `peel_to_entry` points to when the leaf is a tree) and #2870 (`./` and `../` revspec paths resolved against the current directory).

This issue was drafted and submitted by an AI assistant (Claude) on behalf of @alpaylan, at their request, from an automated differential test (gix vs git 2.56).
