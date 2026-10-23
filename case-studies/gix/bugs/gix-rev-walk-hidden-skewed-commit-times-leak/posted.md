# `rev_walk().with_hidden()` returns ancestors of hidden commits when commit times are skewed and there is no commit-graph

Posted as https://github.com/GitoxideLabs/gitoxide/issues/3104 on 2026-10-23. The text below is the issue as posted.

---

### Current behavior 😯

In a linear history where one commit is older than its parent, `repo.rev_walk(tips).with_hidden(hidden)` returns a commit that is an ancestor of a hidden commit. This happens only when the repository has no commit-graph file.

History, oldest first, with committer times: `root (t=2) <- skewed (t=0) <- visible (t=3) <- hidden (t=4)`.

```sh
git rev-list visible root ^hidden
# git 2.56: (nothing)
# gix: root
```

### Expected behavior 🤔

The walk should return nothing, because both tips are ancestors of `hidden`. The docs of `Platform::with_hidden` say: "Don't cross the given `tips` (commits) during traversal or return them, and also don't return any of their ancestors."

### Git behavior

```sh
git rev-list visible root ^hidden
# git 2.56: (nothing)
```

### Steps to reproduce 🕹

```sh
git init -q repo && cd repo
c() { GIT_COMMITTER_DATE="$1 +0000" git commit -q --allow-empty -m $2 && git tag $2; }
c 1700000002 root
c 1700000000 skewed      # older than its parent
c 1700000003 visible
c 1700000004 hidden
git rev-list visible root ^hidden
# git 2.56: (nothing)
```

```rust
let repo = gix::open(".")?;
let id = |r: &str| repo.rev_parse_single(r).unwrap().detach();
let walk = repo.rev_walk([id("visible"), id("root")]).with_hidden([id("hidden")]).all()?;
// gix: yields `root`
```

After `git commit-graph write --reachable`, gix returns nothing, like git.

### Notes

- Tested on gitoxide `main` at e87b6eefb (2026-10-22; the current main f984c56f0 only changes a test) and on the released `gix` 0.89.0 from crates.io: affected in both. Reference: git 2.56.0.
- Where: `compute_hidden_frontier` in `gix-traverse/src/commit/simple.rs` (L167) stops once every queued commit is STALE (L192). That is only safe when the queue is in topological order. Without a commit-graph there are no generation numbers, so the queue is ordered by commit time, and with skewed times `root` is taken as visible-only before the hidden paint reaches it.
- The early stop came with f6d993ef0 "fix: faster `with-hidden()`" (#2523). Related but different: #2159 (first-parent mode), #2161 (breadth-first sorting), #2408.

This issue was drafted and submitted by an AI assistant (Claude) on behalf of @alpaylan, at their request, from an automated differential test (gix vs git 2.56).
