**Title:** cut: with `--output-delimiter`, the last field is printed twice when the line has no terminator

### Describe the bug

With `--output-delimiter`, `cut -f` prints the last field of a line a second time when the line has no trailing newline (the last line of a file, or `-z` input without a final NUL) and the field list has a range that reaches the last field followed by another range that starts after it. The extra copy comes after another output delimiter.

The GNU manual (cut invocation) says:

> The list elements can be repeated, can overlap, and can be specified in any order; but the selected input is written in the same order that it is read, and is written exactly once.

GNU coreutils 9.12:

```sh
printf 'a\tb' | cut --output-delimiter=: -f 1-2,3
# GNU 9.12: a:b
# uutils:   a:b:b

printf 'a,b,c' | cut -d, --output-delimiter=: -f 2-3,4-
# GNU 9.12: b:c
# uutils:   b:c:c

printf 'a\tb\n' | cut --output-delimiter=: -f 1-2,3
# GNU 9.12: a:b
# uutils:   a:b
```

The same happens with `-z` (`printf 'a,b' | cut -z -d, -O: -f 1-2,3` prints `a:b:b`). Without `--output-delimiter`, or with a terminated line, the output is right.

**Versions.** uutils main 342b6f7bd10adee39809516f56d86425422cc9ec (2026-10-22) and the 0.12.0 release binary, macOS 26 aarch64, `LC_ALL=C`, compared with GNU coreutils 9.12.

**Where.** `write_fields_line` in `src/uu/cut/src/cut.rs`: when an unterminated line runs out of delimiters, the code leaves only the loop over the current range, and the next range prints the rest of the line again.

Related: none found. #15341 / PR #15345 (an output delimiter starting with `-`) concern the same option but are about parsing it.

Found with an automated differential tester that compares uutils with GNU 9.12. I reproduced it by hand on main and on 0.12.0.

### Reproduction steps

```sh
export LC_ALL=C

printf 'a\tb' | cut --output-delimiter=: -f 1-2,3
# GNU 9.12: a:b
# uutils:   a:b:b
```

I have a patch with tests and can open a PR.
