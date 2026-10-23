# `HEAD^{/text}` on an unborn HEAD searches all refs instead of failing

Posted as https://github.com/GitoxideLabs/gitoxide/issues/3100 on 2026-10-23. The text below is the issue as posted.

---

### Current behavior 😯

If the anchor of `<rev>^{/text}` is a symbolic ref that can't be resolved (unborn `HEAD`, dangling branch symref), gix drops the anchor and searches every ref, like `:/text`:

```
repo.rev_parse_single("HEAD^{/wip}")     # HEAD -> refs/heads/unborn (unborn)
# gix: Ok(<commit "wip" on main>)
```

### Expected behavior 🤔

An error. `<rev>^{/text}` only searches commits reachable from `<rev>` ([gitrevisions(7)](https://git-scm.com/docs/gitrevisions)), and nothing is reachable from a ref that doesn't resolve.

### Git behavior

```sh
git rev-parse --verify 'HEAD^{/wip}'
# git 2.56: fatal: Needed a single revision
```

### Steps to reproduce 🕹

```sh
git init -q -b main repo && cd repo
git commit -q --allow-empty -m wip
git checkout -q --orphan unborn
git rev-parse --verify 'HEAD^{/wip}'
# git 2.56: fatal: Needed a single revision
# gix: repo.rev_parse_single("HEAD^{/wip}") -> Ok(<commit "wip" on main>)
```

### Notes

- Tested on gitoxide `main` at e87b6eefb (2026-10-22; the current main f984c56f0 only changes a test) and on the released `gix` 0.89.0 from crates.io: affected in both. Reference: git 2.56.0.
- Related: #2908 / PR #2911 (empty pattern `^{/}`), a different cause. No existing issue found.
- Code: in `Navigate::find`, peeling the anchor fails with a delayed error and leaves `self.objs[idx]` as `None`; the `None` arm, meant for anchorless `:/text`, then walks all refs and the delayed error is dropped ([navigate.rs#L190](https://github.com/GitoxideLabs/gitoxide/blob/7540f69bf871c851b826437f7efc5d84b5d9962f/gix/src/revision/spec/parse/delegate/navigate.rs#L190)).

This issue was drafted and submitted by an AI assistant (Claude) on behalf of @alpaylan, at their request, from an automated differential test (gix vs git 2.56).
