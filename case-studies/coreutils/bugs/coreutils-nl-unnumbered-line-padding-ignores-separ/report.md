**Title:** nl: unnumbered lines are padded with width+1 spaces, ignoring the length of `-s`

### Describe the bug

For a line that is not numbered (for example an empty line under the default `-b t`), uutils nl writes `number_width + 1` spaces, as if the separator were always the one-byte TAB. With a longer or empty `-s STRING`, unnumbered lines no longer line up with numbered ones. GNU nl pads with `number_width + length(STRING)` spaces.

The GNU manual (nl invocation) says:

> `-s STRING`: Separate the line number from the text line in the output with STRING (default is the TAB character).
>
> When a line is not numbered, the current line number is not incremented, but the line number separator character is still prepended to the line.

GNU 9.12 keeps the text column aligned: the prefix of an unnumbered line is as wide as the number plus the separator.

```sh
printf 'a\n\nb\n' | nl -s ':-:' | sed -n 2p | tr ' ' .
# GNU 9.12: .........      (9 = 6 + 3)
# uutils:   .......        (7 = 6 + 1)

printf 'a\n\nb\n' | nl -s '' | sed -n 2p | tr ' ' .
# GNU 9.12: ......         (6)
# uutils:   .......        (7)
```

With the default TAB separator both print 7 spaces.

**Versions.** uutils main eee56033d28bf7ef5cc816286b048779d57fb803 (2026-10-21) and the 0.12.0 release binary, macOS 26 aarch64 (the loop found it on Linux x86_64), `LC_ALL=C`, compared with GNU coreutils 9.12.

**Where.** `nl()` in `src/uu/nl/src/nl.rs`: `let blank_width = settings.number_width + 1;`.

Found with an automated differential tester that compares uutils with GNU 9.12.

I have a patch with tests and can open a PR.

### Reproduction steps

```sh
export LC_ALL=C
printf 'a\n\nb\n' | nl -s ':-:' | cat -A
# GNU 9.12:
#      1:-:a$
#          $
#      2:-:b$
# uutils:
#      1:-:a$
#        $
#      2:-:b$
```
