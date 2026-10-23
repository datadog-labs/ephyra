# `@{u}@{1}` is rejected with "unconsumed input"; git reads the upstream ref's reflog

Posted as https://github.com/GitoxideLabs/gitoxide/issues/3098 on 2026-10-23. The text below is the issue as posted.

---

### Current behavior 😯

gix accepts only one `@{…}` group after a ref name, so a reflog selector after `@{upstream}` / `@{push}` fails:

```
repo.rev_parse_single("@{u}@{1}")
# gix: Err: unconsumed input, input="@{1}"
```

`main@{upstream}@{1}` and `@{u}@{<date>}` fail the same way.

### Expected behavior 🤔

`@{u}@{1}` is the previous value of the upstream tracking ref (`refs/remotes/origin/main@{1}`), as in git. git's test suite covers it (`t1507-rev-parse-upstream.sh`: "`<branch>@{u}@{1} resolves correctly`").

### Git behavior

```sh
git rev-parse '@{u}@{1}'
# git 2.56: <commit "one">
```

### Steps to reproduce 🕹

```sh
git init -q -b main repo && cd repo
git commit -q --allow-empty -m one
git remote add origin ../origin.git
git update-ref refs/remotes/origin/main HEAD
git commit -q --allow-empty -m two
git update-ref refs/remotes/origin/main HEAD
git branch -q -u origin/main
git rev-parse '@{u}@{1}'
# git 2.56: <commit "one">
# gix: repo.rev_parse_single("@{u}@{1}") -> Err("unconsumed input, input=\"@{1}\"")
```

### Notes

- Tested on gitoxide `main` at e87b6eefb (2026-10-22; the current main f984c56f0 only changes a test) and on the released `gix` 0.89.0 from crates.io: affected in both. Reference: git 2.56.0.
- Related: #3039 / PR #3047 fixed `HEAD@{upstream}`; chaining is not mentioned. No existing issue found.
- Code: `gix-revision`'s `revision()` parses one `@{…}` group, and the sibling-branch arm passes the rest to navigation, which doesn't know `@{` ([function.rs#L547](https://github.com/GitoxideLabs/gitoxide/blob/7540f69bf871c851b826437f7efc5d84b5d9962f/gix-revision/src/spec/parse/function.rs#L547)). The `gix` delegate already handles `sibling_branch()` followed by `reflog()`; only the parser is missing the step.

This issue was drafted and submitted by an AI assistant (Claude) on behalf of @alpaylan, at their request, from an automated differential test (gix vs git 2.56).
