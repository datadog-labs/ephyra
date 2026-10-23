# `status()` reports an executable file replaced by a symlink as `Modified` instead of `TypeChange`

Posted as https://github.com/GitoxideLabs/gitoxide/issues/3111 on 2026-10-23. The text below is the issue as posted.

---

### Current behavior 😯

A committed executable file `a` (mode `100755`) is replaced in the worktree by a symlink.
`Repository::status()` reports `a` as modified (an executable-bit change), not as a type change.

```
gix status (index/worktree items, via Item::summary())
# gix: Modified a
```

With a non-executable file (`100644`) in the same place, gix correctly reports `TypeChange a`.

### Expected behavior 🤔

A type change, as for non-executable files.
The docs of `entry::Mode::change_to_match_fs()` say it "returns the change needed to update this mode to match the file".
For this input it returns `Change::ExecutableBit`, which turns `100755` into `100644`, not into a symlink.

```
gix status (index/worktree items, via Item::summary())
# gix (expected): TypeChange a
```

### Git behavior

```
git status --short
# git 2.56.0:  T a
```

git-status(1): "T = type changed".

### Steps to reproduce 🕹

```sh
git init -b main repo && cd repo
echo 1 > a && chmod +x a
git add a && git commit -m init
rm a && ln -s b a
git status --short   # " T a"
```

Then, on that repository:

```rust
for item in gix::open(".")?.status(gix::progress::Discard)?.into_index_worktree_iter(None)? {
    let item = item?;
    if let Some(s) = item.summary() { println!("{s:?} {}", item.rela_path()); }
}
// prints "Modified a"; git says " T a"
```

### Notes

- Seen on gitoxide `main` at e87b6eefb (2026-10-22, macOS aarch64; the current main f984c56f0 only changes a test) and earlier on 59ba38b89 (2026-10-21, Linux), compared with git 2.56.0.
- Checked on gitoxide `main` at 59ba38b89 (2026-10-21) with git 2.56.0, on Linux.
- Location: `gix-index/src/entry/mode.rs`, `Mode::change_to_match_fs_with_values()`. Only `Mode::FILE if !is_file` falls through to the type-change branch. `Mode::FILE_EXECUTABLE` has no such arm. A symlink is not an executable regular file, so the `FILE_EXECUTABLE if executable_bit && !is_executable` arm matches and returns `Change::ExecutableBit`.
- No related upstream issue found.

This issue was drafted and submitted by an AI assistant (Claude) on behalf of @alpaylan, at their request, from an automated differential test (gix vs git 2.56).
