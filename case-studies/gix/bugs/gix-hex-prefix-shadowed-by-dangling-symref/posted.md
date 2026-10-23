# Dangling symbolic refs win DWIM lookup: short names, short hex prefixes and `@{-1}` fail or resolve to the wrong object

Posted as https://github.com/GitoxideLabs/gitoxide/issues/3090 on 2026-10-23. The text below is the issue as posted.

---

Three cases where a dangling symbolic ref is treated as a match during name lookup. git ignores dangling symrefs there ("warning: ignoring dangling symref ...") and keeps looking; gix stops at them. Each case has its own reproduction below.

### Current behavior 😯

**1. rev-parse of a short name fails when an earlier DWIM candidate is a dangling symbolic ref**

If the first ref that matches a short name is a symbolic ref that can't be resolved, gix stops there and fails, even though a later candidate exists:

```
repo.rev_parse_single("main")     # refs/tags/main -> refs/tags/missing, refs/heads/main exists
# gix: Err: The rev-spec is malformed and misses a ref name: Could not peel 'refs/tags/main' to obtain its target: Reference "refs/tags/missing" could not be found
```

A symref that points to itself gives "Aborting symbolic reference cycle" the same way.

**2. A short object prefix fails to resolve when a dangling symbolic ref has the same name**

If `refs/heads/457be0c` is a symbolic ref to a missing branch, and `457be0c` is also a unique prefix of a commit, gix fails:

```
repo.rev_parse_single("457be0c")
# gix: Err: The rev-spec is malformed and misses a ref name: Could not peel 'refs/heads/457be0c' to obtain its target: Reference "refs/heads/missing" could not be found
```

**3. `@{-1}` still resolves to a deleted branch when its name matches a dangling symref (gap after #2609)**

Since #2609, `@{-1}` fails when the previously checked-out branch was deleted. But if the deleted name still DWIM-matches a dangling symref (here the common dangling `refs/remotes/origin/HEAD`), gix succeeds and returns the old commit from HEAD's reflog:

```
repo.rev_parse_single("@{-1}")     # previous branch "origin" was deleted
# gix: Ok(<commit "init">)
```

### Expected behavior 🤔

**1. rev-parse of a short name fails when an earlier DWIM candidate is a dangling symbolic ref**

The unresolvable candidate is skipped and lookup continues with the next rule (`refs/heads/main`), as in git.

**2. A short object prefix fails to resolve when a dangling symbolic ref has the same name**

The dangling symref is ignored and the spec resolves to the object with that prefix, as in git.

**3. `@{-1}` still resolves to a deleted branch when its name matches a dangling symref (gap after #2609)**

The same error as for any deleted previous branch: `Previous checkout 'origin' does not resolve to an existing revision`.

### Git behavior

**1. rev-parse of a short name fails when an earlier DWIM candidate is a dangling symbolic ref**

```sh
git rev-parse --verify main
# git 2.56: warning: ignoring dangling symref refs/tags/main
# git 2.56: <the commit on refs/heads/main>
```

`expand_ref()` in `refs.c` only accepts a candidate that resolves (`RESOLVE_REF_READING`).

**2. A short object prefix fails to resolve when a dangling symbolic ref has the same name**

```sh
git rev-parse --verify 457be0c
# git 2.56: warning: ignoring dangling symref refs/heads/457be0c
# git 2.56: <the commit 457be0c…>
```

**3. `@{-1}` still resolves to a deleted branch when its name matches a dangling symref (gap after #2609)**

```sh
git rev-parse --verify '@{-1}'
# git 2.56: warning: ignoring dangling symref refs/remotes/origin/HEAD
# git 2.56: fatal: Needed a single revision
```

### Steps to reproduce 🕹

**1. rev-parse of a short name fails when an earlier DWIM candidate is a dangling symbolic ref**

```sh
git init -q -b main repo && cd repo
git commit -q --allow-empty -m init
git symbolic-ref refs/tags/main refs/tags/missing
git rev-parse --verify main
# git 2.56: <the commit>  (with the warning above)
# gix: repo.rev_parse_single("main") -> Err("... Could not peel 'refs/tags/main' ...")
```

A more common shape: a dangling `refs/heads/origin` hides `refs/remotes/origin/HEAD`, so `origin` fails.

**2. A short object prefix fails to resolve when a dangling symbolic ref has the same name**

```sh
git init -q -b main repo && cd repo
git commit -q --allow-empty -m init
p=$(git rev-parse --short=7 HEAD)
git symbolic-ref "refs/heads/$p" refs/heads/missing
git rev-parse --verify "$p"
# git 2.56: <the commit>  (with the warning above)
# gix: repo.rev_parse_single(p) -> Err("... Could not peel 'refs/heads/<p>' ...")
```

**3. `@{-1}` still resolves to a deleted branch when its name matches a dangling symref (gap after #2609)**

```sh
git init -q -b main repo && cd repo
git commit -q --allow-empty -m init
git symbolic-ref refs/remotes/origin/HEAD refs/remotes/origin/main   # dangling
git checkout -q -b origin
git checkout -q main
git branch -q -D origin
git rev-parse --verify '@{-1}'
# git 2.56: fatal: Needed a single revision
# gix: repo.rev_parse_single("@{-1}") -> Ok(<commit "init">)
```

### Notes

- Tested on gitoxide `main` at e87b6eefb (2026-10-22) and on the released `gix` 0.89.0 from crates.io: all cases are affected in both. Reference: git 2.56.0.
- **1.** No existing issue found. The upstream test `resolved_ambiguity_does_not_hide_a_missing_symbolic_referent` expects an error when a dangling alias is the only candidate; git fails there too, so it doesn't conflict.
  - Code: `Delegate::find_ref` stores the first match of `repo.refs.find(name)` without checking it can be followed ([revision.rs#L19](https://github.com/GitoxideLabs/gitoxide/blob/7540f69bf871c851b826437f7efc5d84b5d9962f/gix/src/revision/spec/parse/delegate/revision.rs#L19)). The fix could also go into `gix-ref`'s `try_find`, which would affect all callers.
- **2.** No existing issue found. Same idea as the plain-name case (a dangling symref should not win DWIM lookup), but a different code path: fixing `find_ref` alone does not fix this.
  - Code: `Delegate::disambiguate_prefix` takes any `refs.find()` match, including a symref that can't be followed, and drops the object candidates ([revision.rs#L73](https://github.com/GitoxideLabs/gitoxide/blob/7540f69bf871c851b826437f7efc5d84b5d9962f/gix/src/revision/spec/parse/delegate/revision.rs#L73)).
- **3.** Related: #2609 / PR #2610 (`fix: reject deleted prior checkout branches`) removed the reflog fallback for the not-found case only. No follow-up issue found.
  - Code: in `Delegate::nth_checked_out_branch`, the `Ok(r)` arm does `r.peel_to_id().map_or(id, …)`, so a failed peel falls back to the reflog's old id ([revision.rs#L206](https://github.com/GitoxideLabs/gitoxide/blob/7540f69bf871c851b826437f7efc5d84b5d9962f/gix/src/revision/spec/parse/delegate/revision.rs#L206)).

This issue was drafted and submitted by an AI assistant (Claude) on behalf of @alpaylan, at their request, from an automated differential test (gix vs git 2.56).
