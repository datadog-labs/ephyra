# `@{push}` resolution ignores git's fallbacks: no default `origin`, and `push.default=upstream` maps through the push remote

Posted as https://github.com/GitoxideLabs/gitoxide/issues/3096 on 2026-10-23. The text below is the issue as posted.

---

Two cases where `@{push}` resolves differently from git, both in `gix/src/repository/config/branch.rs`. Each has its own reproduction below.

### Current behavior 😯

**1. `@{push}` fails when the branch has no configured remote; git falls back to `origin`**

With `push.default=current` and no `branch.main.remote`, `branch.main.pushRemote` or `remote.pushDefault`, gix finds no push branch:

```
repo.rev_parse_single("@{push}")
# gix: Err: Branch named refs/heads/main does not have a push tracking branch configured: Couldn't find sibling of Push
```

`branch_remote_tracking_ref_name(name, Direction::Push)` and `branch_remote_ref_name(name, Direction::Push)` return `None` here.

**2. `@{push}` with `push.default=upstream` and a separate push remote maps through the wrong remote**

In a triangular setup (fetch from `up`, `remote.pushDefault=origin`) with `push.default=upstream`, gix resolves `@{push}` to `refs/remotes/origin/main`:

```
repo.rev_parse_single("@{push}")
# gix: <commit "one"> (refs/remotes/origin/main)
repo.rev_parse_single("@{upstream}")
# gix: <commit "two"> (refs/remotes/up/main)
```

### Expected behavior 🤔

**1. `@{push}` fails when the branch has no configured remote; git falls back to `origin`**

`@{push}` resolves to `refs/remotes/origin/main`, as in git: `git push` uses `origin` when nothing is configured. The doc of `branch_remote_tracking_ref_name` describes this exact case: "with `name` being `main` and no setup at all ... `refs/remotes/origin/main` is returned".

**2. `@{push}` with `push.default=upstream` and a separate push remote maps through the wrong remote**

With `push.default=upstream`, git defines `@{push}` as `@{upstream}`: `refs/remotes/up/main`.

### Git behavior

**1. `@{push}` fails when the branch has no configured remote; git falls back to `origin`**

```sh
git rev-parse --symbolic-full-name '@{push}'
# git 2.56: refs/remotes/origin/main
```

**2. `@{push}` with `push.default=upstream` and a separate push remote maps through the wrong remote**

```sh
git rev-parse --symbolic-full-name '@{push}'
# git 2.56: refs/remotes/up/main
```

`branch_get_push_1()` in `remote.c`: `case PUSH_DEFAULT_UPSTREAM: return xstrdup_or_null(branch_get_upstream(branch, err));`

### Steps to reproduce 🕹

**1. `@{push}` fails when the branch has no configured remote; git falls back to `origin`**

```sh
git init -q -b main repo && cd repo
git commit -q --allow-empty -m init
git remote add origin ../origin.git
git update-ref refs/remotes/origin/main HEAD
git config push.default current
git rev-parse '@{push}'
# git 2.56: <commit "init"> (refs/remotes/origin/main)
# gix: repo.rev_parse_single("@{push}") -> Err("... does not have a push tracking branch configured ...")
```

With git's default `push.default=simple`, git fails here too ("no upstream configured"), so the setting matters.

**2. `@{push}` with `push.default=upstream` and a separate push remote maps through the wrong remote**

```sh
git init -q -b main repo && cd repo
git commit -q --allow-empty -m one
git commit -q --allow-empty -m two
git remote add up ../up.git
git remote add origin ../origin.git
git update-ref refs/remotes/up/main HEAD
git update-ref refs/remotes/origin/main HEAD~1
git branch -q -u up/main
git config remote.pushDefault origin
git config push.default upstream
git rev-parse '@{push}'
# git 2.56: <commit "two"> (refs/remotes/up/main)
# gix: repo.rev_parse_single("@{push}") -> <commit "one"> (refs/remotes/origin/main)
```

### Notes

- Tested on gitoxide `main` at e87b6eefb (2026-10-22; the current main f984c56f0 only changes a test) and on the released `gix` 0.89.0 from crates.io: affected in both. Reference: git 2.56.0.
- **1.** Related: #3039 / PR #3047 (`HEAD@{push}` with configured tracking), a different case. No existing issue found.
  - Code: the `Direction::Push` paths in `branch_remote_ref_name` / `branch_remote_tracking_ref_name` use `branch_remote()`, which returns `None` without config; `remote_default_name(Direction::Push)` already has git's fallback ([branch.rs#L59](https://github.com/GitoxideLabs/gitoxide/blob/7540f69bf871c851b826437f7efc5d84b5d9962f/gix/src/repository/config/branch.rs#L59)).
- **2.** Related: #3039 / PR #3047 (`HEAD@{push}`), a different case. No existing issue found.
  - Code: `branch_remote_tracking_ref_name(name, Push)` gets `branch.main.merge` (a ref on the fetch remote `up`) and then maps it through the fetch refspecs of the push remote `origin` ([branch.rs#L113-L139](https://github.com/GitoxideLabs/gitoxide/blob/7540f69bf871c851b826437f7efc5d84b5d9962f/gix/src/repository/config/branch.rs#L113-L139)). Correct only when both remotes are the same.
  - Not tested: `push.default=simple` in a triangular setup, which git treats like `current`.

This issue was drafted and submitted by an AI assistant (Claude) on behalf of @alpaylan, at their request, from an automated differential test (gix vs git 2.56).
