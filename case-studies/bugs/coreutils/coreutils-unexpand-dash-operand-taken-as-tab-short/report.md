**Title:** expand/unexpand: a bare `-` operand is taken as an obsolete `-N` tab option, so standard input is skipped

### Describe the bug

`expand` and `unexpand` treat a bare `-` as an empty obsolete `-N[,M]` tab-stop option instead of the standard-input operand. As a result:

- `expand - FILE` and `unexpand - FILE` never read standard input; they print only FILE and exit 0.
- In `unexpand`, the obsolete option also turns on `--first-only`, so `unexpand -t N -` converts only leading blanks, and `unexpand --first-only -` fails with "the argument '--first-only' cannot be used multiple times".

With `-` as the only operand the output usually looks right, because stdin is read by default anyway.

The GNU manual (expand invocation, and the same sentence in unexpand invocation) says:

> `expand` writes the contents of each given FILE, or standard input if none are given or for a FILE of `-`, to standard output.

and for `unexpand -t`: "This option implies the `-a` option." GNU coreutils 9.12:

```sh
printf 'stdin\n' | unexpand - <(printf 'file\n')
# GNU 9.12: stdin / file
# uutils:   file

printf 'stdin\n' | expand - <(printf 'file\n')
# GNU 9.12: stdin / file
# uutils:   file

printf 'a  b\n' | unexpand -t 3 - | od -An -c
# GNU 9.12:  a  \t   b  \n
# uutils:    a           b  \n
```

**Versions.** uutils main 342b6f7bd10adee39809516f56d86425422cc9ec (2026-10-22) and the 0.12.0 release binary, macOS 26 aarch64, `LC_ALL=C`, compared with GNU coreutils 9.12.

**Where.** `expand_shortcuts()` in `src/uu/unexpand/src/unexpand.rs` and in `src/uu/expand/src/expand.rs` accepts any argument that starts with `-` and has only digits or commas after it. For a bare `-` the rest is empty, so it matches; `unexpand` then also adds `--first-only`.

Related: the `unexpand --first-only -` row in #15254 is this bug. Open PR #14291 changes `expand_shortcuts()` for hyphen-leading tab lists after `-t`, but a bare `-` elsewhere still matches there. #11091 (no output for interactive stdin) is a different problem.

Found with an automated differential tester that compares uutils with GNU 9.12. I reproduced it by hand on main and on 0.12.0.

### Reproduction steps

```sh
export LC_ALL=C
cd "$(mktemp -d)"

printf 'file\n' > f
printf 'stdin\n' | unexpand - f
# GNU 9.12: stdin, then file
# uutils:   file
```

I have a patch with tests and can open a PR.
