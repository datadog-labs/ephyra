**Title:** nl: header/footer section is reset to body at the start of each input file

### Describe the bug

GNU nl treats all its input files as one document, so a header or footer section started in one file continues into the next. uutils nl goes back to the body section at the start of every file, so the first lines of the next file are numbered with the body style (`-b`, default `t`) instead of the header/footer style (default `n`, no numbers).

The GNU manual (nl invocation) says:

> nl treats all of the input files as a single document; it does not reset line numbers or logical pages between files.

```sh
printf 'a\n\\:\n' > f1; printf 'x\n' > f2; nl f1 f2 | cat -A
# GNU 9.12:
#      1^Ia$
# $
#        x$       (x is still in the footer section: not numbered)
# uutils:
#      1^Ia$
# $
#      1^Ix$      (x is numbered as body)
```

The second line (`\:`) is a footer delimiter and prints as an empty line.

**Versions.** uutils main eee56033d28bf7ef5cc816286b048779d57fb803 (2026-10-21) and the 0.12.0 release binary, macOS 26 aarch64 (the loop found it on Linux x86_64), `LC_ALL=C`, compared with GNU coreutils 9.12.

**Where.** `nl()` in `src/uu/nl/src/nl.rs` runs once per file and keeps the current section in a local (`let mut current_numbering_style = &settings.body_numbering;`). The line number already lives in the shared `Stats` and carries over correctly; only the section is lost.

Found with an automated differential tester that compares uutils with GNU 9.12.

I have a patch with tests and can open a PR.

### Reproduction steps

```sh
export LC_ALL=C
cd "$(mktemp -d)"
printf 'a\n\\:\n' > f1; printf 'x\n' > f2
nl f1 f2 | cat -A
# GNU 9.12:
#      1^Ia$
# $
#        x$
# uutils:
#      1^Ia$
# $
#      1^Ix$
```
