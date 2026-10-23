# SHA-256 repositories: loose objects are never found by an abbreviated id

Posted as https://github.com/GitoxideLabs/gitoxide/issues/3101 on 2026-10-23. The text below is the issue as posted.

---

### Current behavior 😯

In a `--object-format=sha256` repository, a short prefix of a loose object doesn't resolve:

```
repo.rev_parse_single("78c8c0d")
# gix: Err: An object prefixed 78c8c0d could not be found
```

The full 64-char id and `HEAD` work. Packed objects are found. When a packed and a loose object share a prefix, gix sees only the packed one and can pick a different object than git.

### Expected behavior 🤔

The prefix resolves to the unique object it names ([gitrevisions(7)](https://git-scm.com/docs/gitrevisions): "a leading substring that is unique within the repository").

### Git behavior

```sh
git rev-parse 78c8c0d
# git 2.56: 78c8c0d522d13dffc691be8b0446745f6d09988853317619f46c7dc90d44e8d5   (ids here come from a run with pinned dates)
```

### Steps to reproduce 🕹

```sh
git init -q --object-format=sha256 -b main repo && cd repo
git commit -q --allow-empty -m init
git rev-parse "$(git rev-parse --short HEAD)"
# git 2.56: <the commit>
# gix: repo.rev_parse_single(<short id>) -> Err("An object prefixed ... could not be found")
```

(`gix` built with the `sha256` feature.)

### Notes

- Tested on gitoxide `main` at e87b6eefb (2026-10-22; the current main f984c56f0 only changes a test) and on the released `gix` 0.89.0 from crates.io: affected in both. Reference: git 2.56.0.
- No existing issue found. Related only loosely: #673 (panics in loose `lookup_prefix`), SHA-256 work in #2581 / #2653.
- Code: `loose::Store::lookup_prefix` uses `prefix.as_oid().kind().len_in_hex()` as the expected file-name length. A prefix of 40 hex chars or fewer is classified as SHA-1, so every 64-char loose name is filtered out ([find.rs#L37](https://github.com/GitoxideLabs/gitoxide/blob/7540f69bf871c851b826437f7efc5d84b5d9962f/gix-odb/src/store_impls/loose/find.rs#L37)). Using the store's own `object_hash` fixes it.

This issue was drafted and submitted by an AI assistant (Claude) on behalf of @alpaylan, at their request, from an automated differential test (gix vs git 2.56).
