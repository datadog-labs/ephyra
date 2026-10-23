# `Commit::describe()` ignores annotated tags that point to another tag

Posted as https://github.com/GitoxideLabs/gitoxide/issues/3103 on 2026-10-23. The text below is the issue as posted.

---

### Current behavior 😯

With the default name selection (`SelectRef::AnnotatedTags`), `Commit::describe()` skips every annotated tag whose target is another tag object (a nested tag, as created by `git tag -a v2 v1`). If an older tag names the same commit, gix returns the older tag. If the nested tag is the only tag, gix finds no name.

```sh
git describe
# git 2.56: v2
# gix: v1
git tag -d v1 && git describe
# git 2.56: v2
# gix: None
```

### Expected behavior 🤔

`v2` is an annotated tag that peels to HEAD and is newer than `v1`, so `describe()` should return `v2` in both cases. `SelectRef::AllTags` already peels such tags fully; only the default mode skips them.

### Git behavior

`git describe` peels the nested tag to its commit. git-describe(1): "tags with newer dates will always be preferred over tags with older dates."

```sh
git describe
# git 2.56: v2
git tag -d v1 && git describe
# git 2.56: v2
```

### Steps to reproduce 🕹

```sh
git init -q repo && cd repo
git commit -q --allow-empty -m one
GIT_COMMITTER_DATE='2000000000 +0000' git tag -a -m v1 v1
GIT_COMMITTER_DATE='2000000001 +0000' git tag -a -m v2 v2 v1   # v2 is a tag of the tag v1
git describe
# git 2.56: v2
```

```rust
let repo = gix::open(".")?;
println!("{:?}", repo.head_commit()?.describe().try_format()?.map(|f| f.to_string()));
// gix: Some("v1")
```

After `git tag -d v1`, git still prints `v2` and gix prints `None`.

### Notes

- Tested on gitoxide `main` at e87b6eefb (2026-10-22; the current main f984c56f0 only changes a test) and on the released `gix` 0.89.0 from crates.io: affected in both. Reference: git 2.56.0.
- Where: `gix/src/commit/mod.rs` L160, in the `SelectRef::AnnotatedTags` branch of `names()`: `tag.target_id().ok()?.object().ok()?.try_into_commit().ok()?.id`. `try_into_commit()` fails when the target is a tag, and the `filter_map` drops that ref without a message.
- Related but different: #503 (describe with max-candidates 0).

This issue was drafted and submitted by an AI assistant (Claude) on behalf of @alpaylan, at their request, from an automated differential test (gix vs git 2.56).
