**Title:** fold: control characters such as `\f`, ESC and DEL take no column, so long lines are not folded

### Describe the bug

In the default column mode, uutils fold counts ASCII control characters other than TAB, backspace and CR (for example `\001`, `\a`, `\v`, `\f`, ESC, DEL) as zero columns. GNU fold counts each of them as one column, so uutils misses breaks that GNU makes. With `-c`, a control character that exactly fills the line also makes uutils write a newline that is not in the input.

The GNU manual (fold invocation) says:

> fold counts screen columns by default; thus, a tab may count more than one column, backspace decreases the column count, and carriage return sets the column to zero.

Only tab, backspace and carriage return are special; every other byte is one column.

```sh
printf 'ab\033cd' | fold -w2 | od -An -c
# GNU 9.12:   a   b  \n 033   c  \n   d
# uutils:     a   b 033  \n   c   d

printf '\f0' | fold -w1 | od -An -c
# GNU 9.12:  \f  \n   0
# uutils:    \f   0

printf 'a\001' | fold -c -w2 | od -An -c
# GNU 9.12:   a 001
# uutils:     a 001  \n        (extra newline at end of input)
```

**Versions.** uutils main eee56033d28bf7ef5cc816286b048779d57fb803 (2026-10-21) and the 0.12.0 release binary, macOS 26 aarch64 (the loop found it on Linux x86_64), `LC_ALL=C`, compared with GNU coreutils 9.12.

**Where.** `process_ascii_line` in `src/uu/fold/src/fold.rs`: the arm for control bytes increments the column count only in `-c` mode, and checks for a full line after pushing the byte rather than before. The UTF-8 path (`process_utf8_chars`, `compute_col_count`) gets 0 from `UnicodeWidthChar::width` for these characters.

Related: #2636 and the `fold --wi 2 unicode.txt` row in #15254 are about multibyte width, not ASCII control characters.

Found with an automated differential tester that compares uutils with GNU 9.12.

I have a patch with tests and can open a PR.

### Reproduction steps

```sh
export LC_ALL=C
printf 'ab\033cd' | fold -w2 | od -An -c
# GNU 9.12:   a   b  \n 033   c  \n   d
# uutils:     a   b 033  \n   c   d

printf 'a\001' | fold -c -w2 | od -An -c
# GNU 9.12:   a 001
# uutils:     a 001  \n
```
