**Title:** expand: tab stops are misplaced after a file without a trailing newline, and on lines longer than 8 KiB

### Describe the bug

`expand` loses track of the output column at two boundaries, so later tabs are expanded as if the line started there:

1. **Between input files.** The column, and the `-i` "still in leading blanks" state, restart at 0 for every FILE. GNU expand treats the files as one stream, so when a file does not end in a newline, the line continues in the next file.
2. **Between read buffers of one file.** When an 8 KiB read buffer contains no tab, the fast path writes it unchanged but does not advance the column. A tab later on the same line is placed as if the line started at the buffer boundary. This needs only one input on stdin, and the output depends on where `read()` happens to split it.

GNU coreutils 9.12:

```sh
printf 'a' > f1; printf '\tb\n' > f2

expand f1 f2 | od -An -c
# GNU 9.12:  a + 7 spaces + b \n      (the tab is at column 1; the next stop is 8)
# uutils:    a + 8 spaces + b \n

expand -i f1 f2 | od -An -c
# GNU 9.12:  a \t b \n                (the tab follows 'a', so it is not initial and stays a tab)
# uutils:    a + 8 spaces + b \n

{ head -c 8190 /dev/zero | tr '\0' a; printf 'bcd\tx\n'; } > big
expand -t3 < big | tail -c 8 | od -An -c
# GNU 9.12:  b c d + 3 spaces + x \n  (the tab is at column 8193; the next stop is 8196)
# uutils:    b c d + 2 spaces + x \n
```

The third case is wrong regardless of GNU: the same line gives different output depending on where an 8 KiB boundary falls.

**Versions.** uutils main 342b6f7bd10adee39809516f56d86425422cc9ec (2026-10-22) and the 0.12.0 release binary, macOS 26 aarch64, `LC_ALL=C`, compared with GNU coreutils 9.12. The per-file reset is old (`col` already starts at 0 for each file in the 2020 code); the buffer case came with 0400116af ("expand: use chunking read calls", first released in 0.7.0).

**Where.** In `src/uu/expand/src/expand.rs`, `expand_file()` starts every file with `let mut col = 0;`, and `expand_buf()` starts every buffer with `let mut init = true;`. The no-tab fast path in `expand_buf()` sets `*col` only when the buffer contains a newline, so a buffer without one leaves the column where it was.

Related: `unexpand` has the same per-file reset (separate issue). None found upstream.

Found with an automated differential tester that compares uutils with GNU 9.12. I reproduced it by hand on main and on 0.12.0.

### Reproduction steps

```sh
export LC_ALL=C
cd "$(mktemp -d)"

printf 'a' > f1; printf '\tb\n' > f2
expand f1 f2 | od -An -c
# GNU 9.12:  a + 7 spaces + b \n
# uutils:    a + 8 spaces + b \n
```

I have a patch with tests and can open a PR.
