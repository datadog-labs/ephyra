# `@{-1}` peels an annotated tag that git returns unpeeled

Posted as https://github.com/GitoxideLabs/gitoxide/issues/3095 on 2026-10-23. The text below is the issue as posted.

---

### Current behavior 😯

If the name of the previous checkout now refers to an annotated tag, `@{-1}` returns the tagged commit, while the same name on its own returns the tag object:

```
repo.rev_parse_single("@{-1}")
# gix: <commit "init">
repo.rev_parse_single("main")
# gix: <tag object "main">
```

### Expected behavior 🤔

`@{-1}` resolves like its checkout name: the tag object. Peeling should only happen with an explicit `^{}`.

### Git behavior

```sh
git rev-parse '@{-1}'
# git 2.56: <tag object "main">
git rev-parse main
# git 2.56: <tag object "main">
```

git replaces `@{-N}` with the checkout name and resolves it with `repo_dwim_ref`, which does not peel.

### Steps to reproduce 🕹

```sh
git init -q -b main repo && cd repo
git commit -q --allow-empty -m init
git checkout -q -b topic
git branch -q -D main
git tag -a -m t main
git rev-parse '@{-1}'
# git 2.56: <tag object>
# gix: repo.rev_parse_single("@{-1}") -> <commit "init">
```

### Notes

- Related: #3090 case 3 is in the same `Ok(r)` arm of `Delegate::nth_checked_out_branch` (falling back when the peel fails). This one is a separate defect: a fix for either one does not fix the other.
- Tested on gitoxide `main` at e87b6eefb (2026-10-22; the current main f984c56f0 only changes a test) and on the released `gix` 0.89.0 from crates.io: affected in both. Reference: git 2.56.0.
- Related: #2609 / PR #2610 touched the same code (deleted branches, not tags). No existing issue found.
- Code: `Delegate::nth_checked_out_branch` calls `r.peel_to_id()`, which also peels tags; plain names only follow symrefs ([revision.rs#L206](https://github.com/GitoxideLabs/gitoxide/blob/7540f69bf871c851b826437f7efc5d84b5d9962f/gix/src/revision/spec/parse/delegate/revision.rs#L206)).

This issue was drafted and submitted by an AI assistant (Claude) on behalf of @alpaylan, at their request, from an automated differential test (gix vs git 2.56).
