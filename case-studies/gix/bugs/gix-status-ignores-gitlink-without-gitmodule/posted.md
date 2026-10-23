# `gix status` ignores a gitlink whose path is not listed in `.gitmodules`

Posted as https://github.com/GitoxideLabs/gitoxide/issues/3114 on 2026-10-23. The text below is the issue as posted.

---

### Current behavior 😯

`gix status` says nothing about a gitlink (an index entry with mode 160000) when its path has no entry in
`.gitmodules`. This happens even when the repository checked out at that path has moved to another commit.
The same check feeds `Repository::is_dirty()`, so that should also return `false` here.

You get such a gitlink when you `git add` an embedded repository. git warns about this but allows it.

```
$ gix status
# gix: (no output)
$ gix status --submodules all
# gix: (no output)
```

### Expected behavior 🤔

`gix status` should report `d` as modified, as it already does when `.gitmodules` lists `d`.
The commit recorded in the index for `d` is not the nested repository's `HEAD`, so the worktree is not clean.

### Git behavior

git reports the entry as a submodule whose commit changed, whether or not `.gitmodules` lists it:

```
$ git status --short
# git: " M d"
$ git status --porcelain=v2
# git: "1 .M SC.. 160000 160000 160000 <id> <id> d"
```

git-status(1) on the v2 `<sub>` field: "'S<c><m><u>' when the entry is a submodule. <c> is 'C' if the
commit changed".

### Steps to reproduce 🕹

```
git init repo && cd repo
git init d && git -C d commit --allow-empty -m one
git add d                 # embedded repo, no .gitmodules entry
git commit -m add
git -C d commit --allow-empty -m two

git status --short        # git: " M d"
gix status                # gix: (nothing)
```

Control: add `.gitmodules` with `submodule.d.path = d` and commit it, and `gix status` prints ` M d`.

### Notes

- Seen on gitoxide `main` at e87b6eefb (2026-10-22, macOS aarch64; the current main f984c56f0 only changes a test) and earlier on 59ba38b89 (2026-10-21, Linux), compared with git 2.56.0.
  configuration.
- Where: `BuiltinSubmoduleStatus::status` in `gix/src/status/index_worktree.rs` returns `Ok(None)` ("no
  change") for every gitlink path that is not in the list built from `repo.submodules()`. That list comes
  from `.gitmodules`. This early return looks like it has been there since 4a4989d51 ("Add submodule support
  for status iterator").
- Related but different: #2587 (is_dirty false negative, unborn HEAD), and #2585/#2490 (false "modified"
  for listed submodules).

This issue was drafted and submitted by an AI assistant (Claude) on behalf of @alpaylan, at their request, from an automated differential test (gix vs git 2.56).
