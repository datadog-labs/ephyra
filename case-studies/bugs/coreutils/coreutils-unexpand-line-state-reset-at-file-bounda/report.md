**Title:** unexpand: a line that continues from one FILE into the next is handled as two lines

### Describe the bug

`unexpand` starts every FILE operand at column 0, as a new line. When a file does not end with a newline, its last line goes on in the next file. GNU `unexpand` handles this the same as `cat f1 f2 | unexpand`. uutils handles the rest of the line as if it were at the start of a line, so blanks that GNU turns into a tab are left as spaces.

The GNU manual (unexpand invocation) says:

> unexpand writes the contents of each given file, or standard input if none are given or for a file of '-', to standard output, converting blanks at the beginning of each line into as many tab characters as needed.

In the first example below, the output line starts with 8 blanks. GNU makes them one tab; uutils does not. GNU coreutils 9.12:

```sh
printf '    ' > a; printf '    x\n' > b
unexpand a b | sed -n l
# GNU 9.12: \tx$
# uutils:           x$

printf 'abcdef' > c; printf '  x\n' > d
unexpand -a c d | sed -n l
# GNU 9.12: abcdef\tx$
# uutils:   abcdef  x$
```

With uutils, `cat a b | unexpand` and `cat c d | unexpand -a` print the GNU output.

**Versions.** uutils main 342b6f7bd10adee39809516f56d86425422cc9ec (2026-10-22) and the 0.12.0 release binary, macOS 26 aarch64, `LC_ALL=C`, compared with GNU coreutils 9.12.

**Where.** `unexpand_file()` in `src/uu/unexpand/src/unexpand.rs` creates a new `PrintState` for each file and writes out pending blanks at the end of each file.

Related: #15656 is the same per-file reset in `expand`. #5852 (closed) added multi-file input. Open PR #15293 (expand, unexpand: flush output after each line) changes `unexpand_file()` but keeps the per-file `PrintState`, so it does not seem to change this.

Found with an automated differential tester that compares uutils with GNU 9.12. I reproduced it by hand on main and on 0.12.0.

### Reproduction steps

```sh
export LC_ALL=C
cd "$(mktemp -d)"

printf '    ' > a; printf '    x\n' > b
unexpand a b | sed -n l
# GNU 9.12: \tx$
# uutils:           x$
```

I have a patch with tests and can open a PR.
