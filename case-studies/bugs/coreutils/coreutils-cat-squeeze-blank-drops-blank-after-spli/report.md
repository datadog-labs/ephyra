**Title:** cat: `-s` drops a single blank line when the line before it was continued from an earlier file or read

### Describe the bug

`cat -s` deletes a blank line that is not repeated. This happens when the previous file (or the previous read from a pipe) ends in the middle of a line. The newline that finishes that line is counted as a blank line, so the real blank line after it is squeezed away.

The GNU manual (cat invocation) says:

> `-s`, `--squeeze-blank`: Suppress repeated adjacent blank lines; output just one empty line instead of several.

Here there is only one blank line, and it disappears. GNU coreutils 9.12 keeps it:

```sh
printf a > f; printf '\n\nb\n' | cat -s f - | od -An -c
# GNU 9.12:  a  \n  \n   b  \n
# uutils:    a  \n   b  \n

(printf a; sleep 0.3; printf '\n\nb\n') | cat -s
# GNU 9.12: a / (empty line) / b
# uutils:   a / b
```

When the same bytes arrive in one read, uutils keeps the blank line (`printf 'a\n\nb\n' | cat -s` prints `a`, an empty line, `b`), so the result depends only on where a file or read boundary falls.

**Versions.** uutils main 342b6f7bd10adee39809516f56d86425422cc9ec (2026-10-22) and the 0.12.0 release binary, macOS 26 aarch64, `LC_ALL=C`, compared with GNU coreutils 9.12.

**Where.** `write_new_line()` in `src/uu/cat/src/cat.rs` sets `state.one_blank_kept` for a newline at the start of a buffer even when `state.at_line_start` is false, that is, when the newline ends a non-empty line.

Related issues: none found.

Found with an automated differential tester that compares uutils with GNU 9.12. I reproduced it by hand on main and on 0.12.0.

### Reproduction steps

```sh
export LC_ALL=C
cd "$(mktemp -d)"

printf a > f1
printf '\n\nb\n' > f2
cat -s f1 f2
# GNU 9.12: a, an empty line, b
# uutils:   a, b
```

I have a patch with tests and can open a PR.
