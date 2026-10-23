# Reflog specs (`<ref>@{N}`, `@{0}`, `HEAD@{n}`) disagree with git in four cases (follow-up to #3040 / #3047)

Posted as https://github.com/GitoxideLabs/gitoxide/issues/3089 on 2026-10-23. The text below is the issue as posted.

---

Four cases where `Delegate::reflog` resolves a reflog spec differently from git's `read_ref_at()` / `repo_dwim_log()`. They are in the same function, so they are reported together; each case has its own reproduction below.

### Current behavior 😯

**1. `rev_parse_single("<ref>@{N}")` fails when the reflog has exactly N entries**

If a ref's reflog has exactly N entries, gix rejects `<ref>@{N}` as out of range:

```
repo.rev_parse_single("main@{1}")
# gix: Err: Reference 'refs/heads/main' has 1 ref-log entries and entry number 1 is out of range
```

**2. `HEAD@{n}` resolves to the null object id when the reflog entry records a ref deletion**

If `HEAD@{n}` selects a reflog entry whose new id is `0000…` (the branch was deleted while checked out), gix returns the null id:

```
repo.rev_parse_single("HEAD@{1}")
# gix: Ok(0000000000000000000000000000000000000000)
```

Anything chained after it then fails, for example `HEAD@{1}^{/x}`: "An object with id 0000… could not be found".

**3. `<ref>@{0}` returns the newest reflog entry instead of the ref's current value**

For a symbolic ref with its own reflog whose target moved later, `alias@{0}` returns the value logged when the symref was created, while `alias` returns the current value:

```
repo.rev_parse_single("alias")
# gix: <id of "two">
repo.rev_parse_single("alias@{0}")
# gix: <id of "one">
```

A date after the newest entry (`alias@{now}`, `alias@{2030-01-01}`) gives the same stale id.

**4. `HEAD@{0}` on an unborn HEAD (or a dangling symref) returns an entry from its reflog; git fails**

If a symbolic ref has its own reflog but its target doesn't exist, gix still answers `<ref>@{n}` and `<ref>@{<date>}` from that log:

```
repo.rev_parse_single("HEAD@{0}")     # HEAD -> refs/heads/missing
# gix: Ok(<id of "one">)
```

If the only entry is `0000… 0000…` (written when checking out a dangling branch), `HEAD@{0}` and `HEAD@{now}` return the null id `0000000000000000000000000000000000000000`.

### Expected behavior 🤔

**1. `rev_parse_single("<ref>@{N}")` fails when the reflog has exactly N entries**

`main@{1}` resolves to the old id of the oldest reflog entry (the commit `one` below), as in git. That is the "n-th prior value" from [gitrevisions(7)](https://git-scm.com/docs/gitrevisions). This only applies when that old id is not null. `main@{2}` stays an error.

**2. `HEAD@{n}` resolves to the null object id when the reflog entry records a ref deletion**

A revision should never resolve to the null id, because it names no object. git stops at the deletion entry and returns the ref's current value.

**3. `<ref>@{0}` returns the newest reflog entry instead of the ref's current value**

`<ref>@{0}`, and `<ref>@{<date>}` for a date after the newest entry, resolve to the ref's current value. That is the "0th prior value" and "the value of the ref at a prior point in time" from [gitrevisions(7)](https://git-scm.com/docs/gitrevisions).

**4. `HEAD@{0}` on an unborn HEAD (or a dangling symref) returns an entry from its reflog; git fails**

An error, as in git. A ref that doesn't resolve has no value to look up. gix already rejects the short form `@{0}` on an unborn HEAD ("Unborn heads do not have a reflog yet"), but the explicit `HEAD@{0}` skips that check.

### Git behavior

**1. `rev_parse_single("<ref>@{N}")` fails when the reflog has exactly N entries**

```sh
git rev-parse 'main@{1}'
# git 2.56: <id of commit "one">
git rev-parse 'main@{2}'
# git 2.56: fatal: log for 'main' only has 1 entries
```

git does this on purpose: `get_oid_basic()` in `object-name.c` has the case `nth == co_cnt && !is_null_oid(oid)` ("there were only N entries ... it did still fill in the oid with the 'old' value, which we can use").

**2. `HEAD@{n}` resolves to the null object id when the reflog entry records a ref deletion**

```sh
git rev-parse 'HEAD@{1}'
# git 2.56: warning: log for ref HEAD unexpectedly ended on ...
# git 2.56: <id of commit "two" (current HEAD)>
```

**3. `<ref>@{0}` returns the newest reflog entry instead of the ref's current value**

```sh
git rev-parse 'alias@{0}'
# git 2.56: warning: log for ref refs/heads/alias unexpectedly ended on ...
# git 2.56: <id of "two">
```

`read_ref_at()` in `refs.c` starts from the ref's current value and keeps it when the newest log line doesn't match.

**4. `HEAD@{0}` on an unborn HEAD (or a dangling symref) returns an entry from its reflog; git fails**

```sh
git rev-parse --verify 'HEAD@{0}'
# git 2.56: fatal: Needed a single revision
```

`repo_dwim_log()` in `refs.c` skips candidates that don't resolve.

### Steps to reproduce 🕹

**1. `rev_parse_single("<ref>@{N}")` fails when the reflog has exactly N entries**

```sh
git init -q -b main repo && cd repo
git commit -q --allow-empty -m one
git commit -q --allow-empty -m two
git reflog delete 'main@{1}'     # drop the creation entry; one entry "one -> two" is left
git rev-parse 'main@{1}'
# git 2.56: <id of "one">
# gix: repo.rev_parse_single("main@{1}") -> Err(... entry number 1 is out of range)
```

The same happens after `git reflog expire` drops old entries.

**2. `HEAD@{n}` resolves to the null object id when the reflog entry records a ref deletion**

```sh
git init -q -b main repo && cd repo
git commit -q --allow-empty -m one
git update-ref -d refs/heads/main     # HEAD's log gets "<one> 0000…"
git commit -q --allow-empty -m two
git rev-parse 'HEAD@{1}'
# git 2.56: <id of "two">  (with the warning above)
# gix: repo.rev_parse_single("HEAD@{1}") -> Ok(0000000000000000000000000000000000000000)
```

**3. `<ref>@{0}` returns the newest reflog entry instead of the ref's current value**

```sh
git init -q -b main repo && cd repo
git commit -q --allow-empty -m one
git symbolic-ref refs/heads/alias refs/heads/main   # alias gets its own log: 0000… -> one
git commit -q --allow-empty -m two                  # only main's and HEAD's logs are appended
git rev-parse 'alias@{0}'
# git 2.56: <id of "two">
# gix: repo.rev_parse_single("alias@{0}") -> <id of "one">
```

This happens in normal use with `refs/remotes/origin/HEAD`: it gets its own log at clone time, and later fetches only append to `origin/main`'s log. After that, gix resolves `origin@{0}` to the commit from clone time (verified the same way).

**4. `HEAD@{0}` on an unborn HEAD (or a dangling symref) returns an entry from its reflog; git fails**

```sh
git init -q -b main repo && cd repo
git commit -q --allow-empty -m one
git symbolic-ref HEAD refs/heads/missing    # unborn HEAD that still has a reflog
git rev-parse --verify 'HEAD@{0}'
# git 2.56: fatal: Needed a single revision
# gix: repo.rev_parse_single("HEAD@{0}") -> Ok(<id of "one">)
```

A dangling `refs/remotes/origin/HEAD` with its own log shows the same thing with `origin@{0}`.

### Notes

- Tested on gitoxide `main` at e87b6eefb (2026-10-22) and on the released `gix` 0.89.0 from crates.io: all cases are affected in both. Reference: git 2.56.0.
- **1.** Related: #3040 / PR #3047 changed the same function (reflog on symbolic refs) but not the entry-count bound. No existing issue found.
  - Code: the `ReflogLookup::Entry(no)` arm of `Delegate::reflog` ([revision.rs#L150-L160](https://github.com/GitoxideLabs/gitoxide/blob/7540f69bf871c851b826437f7efc5d84b5d9962f/gix/src/revision/spec/parse/delegate/revision.rs#L150-L160)) errors as soon as `it.nth(no)` is `None`.
- **2.** Related: #3040 / PR #3047 (same function, different case). No existing issue found.
  - Code: `Delegate::reflog` returns the selected line's `new_oid` without checking for null, both for `@{n}` and for `@{<date>}` ([revision.rs#L127-L160](https://github.com/GitoxideLabs/gitoxide/blob/7540f69bf871c851b826437f7efc5d84b5d9962f/gix/src/revision/spec/parse/delegate/revision.rs#L127-L160)).
- **3.** Related: #3040 / PR #3047 made a symref without its own log use the target's log, and kept "an existing log still takes precedence". This is the stale-own-log case. No existing issue found.
  - Code: `Delegate::reflog` returns `line.new_oid` for `Entry(0)` and `closest_line.new_oid` for dates ([revision.rs#L127-L160](https://github.com/GitoxideLabs/gitoxide/blob/7540f69bf871c851b826437f7efc5d84b5d9962f/gix/src/revision/spec/parse/delegate/revision.rs#L127-L160)).
- **4.** Related: #3040 / PR #3047. Since then, a symref with no log follows its target (and fails if the target is missing), but a symref with its own log is never checked. No existing issue found.
  - Code: `Delegate::reflog` only calls `follow_to_object()` when `!r.log_exists()` ([revision.rs#L118-L121](https://github.com/GitoxideLabs/gitoxide/blob/7540f69bf871c851b826437f7efc5d84b5d9962f/gix/src/revision/spec/parse/delegate/revision.rs#L118-L121)).

This issue was drafted and submitted by an AI assistant (Claude) on behalf of @alpaylan, at their request, from an automated differential test (gix vs git 2.56).
