# `<name>@{0}` fails with "reflog entries require a ref name" when the branch name looks like a short hash

Posted as https://github.com/GitoxideLabs/gitoxide/issues/3099 on 2026-10-23. The text below is the issue as posted.

---

### Current behavior 😯

If a branch name is also a valid abbreviated object id (branch `457b` while HEAD is `457be0c…`), gix resolves `457b` to the branch, but rejects `457b@{0}`:

```
repo.rev_parse_single("457b")
# gix: Ok(<HEAD commit>)
repo.rev_parse_single("457b@{0}")
# gix: Err: reflog entries require a ref name, input="457b"
```

`457b@{<date>}` fails the same way.

### Expected behavior 🤔

`457b@{0}` reads the reflog of `refs/heads/457b`. gix already treats the short hex name as the ref here (default `RefsHint`, "Git chooses this as default as well"), and only refs have reflogs.

### Git behavior

```sh
git rev-parse '457b@{0}'
# git 2.56: warning: refname '457b' is ambiguous.
# git 2.56: <HEAD commit>
```

### Steps to reproduce 🕹

```sh
git init -q -b main repo && cd repo
git commit -q --allow-empty -m init
b=$(git rev-parse --short=4 HEAD)    # branch named like HEAD's 4-char prefix
git branch "$b"
git rev-parse "$b@{0}"
# git 2.56: <HEAD commit>
# gix: repo.rev_parse_single("$b@{0}") -> Err("reflog entries require a ref name")
```

### Notes

- Tested on gitoxide `main` at e87b6eefb (2026-10-22; the current main f984c56f0 only changes a test) and on the released `gix` 0.89.0 from crates.io: affected in both. Reference: git 2.56.0.
- No existing issue found.
- Code: in `gix-revision`'s `revision()`, a hex-looking name goes through `try_set_prefix`; the delegate then switches to the ref of the same name, but `has_ref_or_implied_name` is only set on the `find_ref` path, so the `@{…}` arm bails ([function.rs#L543](https://github.com/GitoxideLabs/gitoxide/blob/7540f69bf871c851b826437f7efc5d84b5d9962f/gix-revision/src/spec/parse/function.rs#L543)). The same check guards `@{u}`/`@{push}` for such names (not compared with git).

This issue was drafted and submitted by an AI assistant (Claude) on behalf of @alpaylan, at their request, from an automated differential test (gix vs git 2.56).
