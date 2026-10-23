# `gix_traverse::commit::topo` returns a tip that an end hides (with `Parents::First`, or when the tip is also an end)

Posted as https://github.com/GitoxideLabs/gitoxide/issues/3105 on 2026-10-23. The text below is the issue as posted.

---

### Current behavior 😯

A topo walk returns a tip that is reachable from one of its ends:

1. With `Parents::First`, when the end reaches the tip only through a merge's second parent. With `Parents::All` the same walk is correct.
2. With any parent mode, when the tip is also an end.

Both happen with `Sorting::TopoOrder` and `Sorting::DateOrder`.

```sh
git rev-list --topo-order --first-parent side ^main
# git 2.56: (nothing)
# gix: side
git rev-list --topo-order main ^main
# git 2.56: (nothing)
# gix: main
```

### Expected behavior 🤔

The walk should return nothing in both cases. The builder docs say it works "Like `git rev-list --topo-order ^ends tips`" (`gix-traverse/src/commit/topo/init.rs` L30).

### Git behavior

In git, `--first-parent` only limits the walk that finds commits to include. The `^` side still follows every parent, unless `--exclude-first-parent-only` is given. git-rev-list(1): "--first-parent: When finding commits to include, follow only the first parent commit upon seeing a merge commit."

```sh
git rev-list --topo-order --first-parent side ^main
# git 2.56: (nothing)
git rev-list --date-order --first-parent side ^main
# git 2.56: (nothing)
git rev-list --topo-order main ^main
# git 2.56: (nothing)
```

### Steps to reproduce 🕹

```sh
git init -q -b main repo && cd repo
git commit -q --allow-empty -m base
git switch -q -c side && git commit -q --allow-empty -m side
git switch -q main && git merge -q --no-ff -m merge side
git rev-list --topo-order --first-parent side ^main
# git 2.56: (nothing)
```

```rust
use gix::traverse::commit::{topo, Parents};
let repo = gix::open(".")?;
let id = |r: &str| repo.rev_parse_single(r).unwrap().detach();
let walk = topo::Builder::from_iters(&repo.objects, [id("side")], Some([id("main")]))
    .parents(Parents::First)
    .build()?;
// gix: yields `side`
```

With `[id("main")]` as both tips and ends (and the default `Parents::All`), the walk yields `main`.

### Notes

- Tested on gitoxide `main` at e87b6eefb (2026-10-22; the current main f984c56f0 only changes a test) and on the released `gix` 0.89.0 from crates.io: affected in both. Reference: git 2.56.0.
- Where: `Builder::build` (`gix-traverse/src/commit/topo/init.rs`, the tip loop at L180) queues every tip whose indegree is 1 without checking `WalkFlags::Uninteresting`. With `Parents::All` the end's walk usually raises the tip's indegree, so the tip is skipped by chance. Also, `explore_walk_step` (`iter.rs` L110) follows only first parents even for uninteresting commits, so the hidden side does not reach second parents.
- Related: #2159 (same symptom in `commit::Simple` with single-parent mode), #2902 / #2903 (topo walk with identical tips).

This issue was drafted and submitted by an AI assistant (Claude) on behalf of @alpaylan, at their request, from an automated differential test (gix vs git 2.56).
