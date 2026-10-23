# `status()` fails when HEAD's branch is a symbolic ref to an unborn branch

Posted as https://github.com/GitoxideLabs/gitoxide/issues/3112 on 2026-10-23. The text below is the issue as posted.

---

### Current behavior 😯

HEAD points to `refs/heads/main`. `refs/heads/main` is itself a symbolic ref to `refs/heads/topic`, which does not exist yet.

`Repository::head()` does not report this as unborn. It returns `Kind::Symbolic(main)`. Then `head_tree_id_or_empty()` tries to peel it and fails. So `Repository::status()` fails too:

```
repo.head()?.is_unborn()
# gix: false
repo.head_tree_id_or_empty()
# gix: Err: The ref partially named "refs/heads/topic" could not be found
repo.status(gix::progress::Discard)?.into_iter(None)
# gix: Err: The ref partially named "refs/heads/topic" could not be found
```

### Expected behavior 🤔

HEAD should be treated as being on the unborn branch `topic`. `head_tree_id_or_empty()` should return the empty tree, and `status()` should work as in a fresh repository.

The docs of `head::Kind::Unborn` say: "The yet-to-be-created reference the symbolic HEAD refers to." `head_tree_id_or_empty()` is documented as returning "an empty tree hash if the repository HEAD is unborn". `status()` is documented as "most similar to `git status --ignored=no`".

```
repo.head()?.is_unborn()
# expected: true, referent refs/heads/topic
repo.head_tree_id_or_empty()
# expected: 4b825dc642cb6eb9a060e54bf8d69288fbee4904 (empty tree)
repo.status(gix::progress::Discard)?.into_iter(None)
# expected: Ok, no items
```

### Git behavior

git follows the chain and treats `topic` as the current, unborn branch:

```
git status
# git: On branch topic
# git: No commits yet
# git: nothing to commit (create/copy files and use "git add" to track)
git symbolic-ref HEAD
# git: refs/heads/topic
git branch --show-current
# git: topic
```

After `git add a`, `git status --short` lists `A  a`. After a commit, both `topic` and `main` point to it.

### Steps to reproduce 🕹

```sh
git init -b main repo
git -C repo symbolic-ref refs/heads/main refs/heads/topic
```

Then:

```rust
gix::open("repo")?.status(gix::progress::Discard)?.into_iter(None)  // Err: ref "refs/heads/topic" could not be found
```

The same error comes from `repo.head_tree_id_or_empty()` and `repo.is_dirty()`.

### Notes

- Seen on gitoxide `main` at e87b6eefb (2026-10-22, macOS aarch64; the current main f984c56f0 only changes a test) and earlier on 59ba38b89 (2026-10-21, Linux), compared with git 2.56.0.
- Checked on main at 59ba38b89a4d (gix 0.89.0) and git 2.56.0.
- Cause: `Repository::head()` (`gix/src/repository/reference.rs:168`) returns `Kind::Unborn` only if HEAD's direct target is missing. It does not look further when that target is itself a symbolic ref to a missing ref.
- Related: #2589 ("Let is_dirty handle unborn HEADs") fixed the case where HEAD's direct target is missing. This is the indirect case.

This issue was drafted and submitted by an AI assistant (Claude) on behalf of @alpaylan, at their request, from an automated differential test (gix vs git 2.56).
