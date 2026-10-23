**Title:** expand: a NUL byte takes one column, so the next tab stop comes one column early

### Describe the bug

uutils expand counts a NUL byte as one column. GNU coreutils 9.12 counts it as zero columns, so every tab after a NUL on the same line gets one space less per NUL in uutils. NUL is the only byte that differs: every other single byte gives the same output in both (checked with `printf 'X\t|' | expand` for all bytes, in `C` and `C.UTF-8`).

GNU expand measures each character by its display width (since 9.11, which added multi-byte support; GNU 9.4 still gives NUL one column, like uutils today). A NUL has zero display width; POSIX `wcwidth()` "shall either return 0 (if wc is a null wide-character code), or return the number of column positions to be occupied by the wide-character code wc". uutils targets the latest GNU release (CONTRIBUTING.md, "Reference version of GNU coreutils").

```sh
printf '\0\t|\n' | expand | cat -v | tr ' ' .
# GNU 9.12: ^@........|
# uutils:   ^@.......|

printf 'ab\0\0cd\tx\n' | expand | cat -v | tr ' ' .
# GNU 9.12: ab^@^@cd....x
# uutils:   ab^@^@cd..x
```

**Versions.** uutils main eee56033d28bf7ef5cc816286b048779d57fb803 (2026-10-21) and the 0.12.0 release binary, macOS 26 aarch64 (the loop found it on Linux x86_64), `LC_ALL=C` (also `C.UTF-8`), compared with GNU coreutils 9.12.

**Where.** `classify_char` in `src/uu/expand/src/expand.rs` returns width 1 for every ASCII byte other than TAB and backspace, NUL included.

`unexpand -a` has the same difference the other way round: `printf '\0       x\n' | unexpand -a` keeps the 7 spaces in GNU 9.12 and writes `\0\tx` in uutils.

Found with an automated differential tester that compares uutils with GNU 9.12.

I have a patch with tests and can open a PR.

### Reproduction steps

```sh
export LC_ALL=C
printf '\0\t|\n' | expand | cat -v | tr ' ' .
# GNU 9.12: ^@........|
# uutils:   ^@.......|
```
