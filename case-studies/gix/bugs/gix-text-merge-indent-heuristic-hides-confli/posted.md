# Text merge reports no conflict when both sides append different lines at the end of a file (diff hunks placed with the indent heuristic)

Posted as https://github.com/GitoxideLabs/gitoxide/issues/3110 on 2026-10-23. The text below is the issue as posted.

---

### Current behavior 😯

Both sides add different lines at the end of the same file. `gix merge file` merges cleanly and puts both insertions one after the other. The tree merge does the same: `gix merge commit` writes the file without conflict markers.

```sh
printf 'a\n\n\nb\n'      > base
printf 'a\n\n\nb\n\nb\n' > ours     # appends "" and "b"
printf 'a\n\n\nb\nc\n'   > theirs   # appends "c"
gix merge file ours base theirs; echo "exit $?"
# gix: a, "", "", b, "", b, c   (no conflict markers)
# gix: exit 0
```

### Expected behavior 🤔

A content conflict, as git reports. Two sides inserted different lines at the same place (the end of the file), and nothing tells the user.

### Git behavior

```sh
git merge-file -p ours base theirs; echo "exit $?"
# git: a
# git: (empty)
# git: (empty)
# git: b
# git: <<<<<<< ours
# git: (empty)
# git: b
# git: =======
# git: c
# git: >>>>>>> theirs
# git: exit 1
```

`git merge-tree --write-tree` on the same change as two commits also prints `CONFLICT (content): Merge conflict in f`. Tested with git 2.56.0.

### Steps to reproduce 🕹

Inside any git repository (`git init`):

```sh
printf 'a\n\n\nb\n'      > base
printf 'a\n\n\nb\n\nb\n' > ours
printf 'a\n\n\nb\nc\n'   > theirs
git merge-file -p ours base theirs   # conflict, exit 1
gix merge file ours base theirs      # clean merge, exit 0
```

The insertion in `ours` (`"" b` after the last line) can also be read as `b ""` inserted before the last `b`. `git diff --indent-heuristic base ours` shows it the second way (`@@ -1,4 +1,6 @@`, `+b` `+`). `git diff --no-indent-heuristic` shows it at the end (`@@ -2,3 +2,5 @@`). git's merge places it at the end, so it touches `theirs`' insertion and conflicts. gix places it one line up, so the two hunks no longer touch and both are taken.

Swapping `ours` and `theirs` gives the same result: git conflicts, gix merges cleanly.

### Notes

- Seen on gitoxide `main` at e87b6eefb (2026-10-22, macOS aarch64; the current main f984c56f0 only changes a test) and earlier on 59ba38b89 (2026-10-21, Linux), compared with git 2.56.0.
- gix built from `main` at 59ba38b89 (2026-10-21). git 2.56.0.
- Where in the code: `collect_hunks()` in `gix-merge/src/blob/builtin_driver/text/utils.rs` calls `diff.postprocess_lines(input)`, which runs imara-diff's indent heuristic. This came in with 91feaf293 ("fix: enable diff-postprocessing in when merging blobs", "This can lead to cleaner, more Git-like merges."). Git's merge does not seem to use the indent heuristic here: its result matches the `--no-indent-heuristic` hunk placement. imara-diff's `postprocess_no_heuristic` ("Move sliders as far down as possible") may be the matching choice.
- `gix-merge/src/tree/function/resolve.rs` calls merge-ort the behavioral reference.
- Related but different: #3076 / #3077 (text merge vs `git merge-file` with runs of equal lines). That fix is already in 59ba38b89, and this case still differs.
- Found by an automated differential test that merges random commit pairs with gix (`merge_commits`) and `git merge-tree`.

This issue was drafted and submitted by an AI assistant (Claude) on behalf of @alpaylan, at their request, from an automated differential test (gix vs git 2.56).
