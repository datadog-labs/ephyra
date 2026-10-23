**Title:** fmt: one space instead of two after a sentence-ending first word at the end of a line

### Describe the bug

When the first word of a paragraph ends in `.`, `?` or `!` and is the last word of its input line, uutils fmt joins the next line onto it with one space. Every later sentence-ending word at the end of a line gets two spaces, as it should.

The GNU manual (fmt invocation) says:

> A "sentence break" is defined as either the end of a paragraph or a word ending in any of '.?!', followed by two spaces or end of line

and for `-u`: "Reduce spacing between words to one space, and spacing between sentences to two spaces."

```sh
printf 'Hello.\nWorld.\n' | fmt
# GNU 9.12: "Hello.  World."
# uutils:   "Hello. World."

printf 'Why?\nnot\n' | fmt -u
# GNU 9.12: "Why?  not"
# uutils:   "Why? not"

printf 'x a.\nz\n' | fmt
# GNU 9.12: "x a.  z"
# uutils:   "x a.  z"        (same word in second position: correct)
```

**Versions.** uutils main eee56033d28bf7ef5cc816286b048779d57fb803 (2026-10-21) and the 0.12.0 release binary, macOS 26 aarch64 (the loop found it on Linux x86_64), `LC_ALL=C`, compared with GNU coreutils 9.12.

**Where.** `break_lines` in `src/uu/fmt/src/linebreak.rs` writes the first word itself and then calls `break_simple` / `break_knuth_plass`, which start with "previous word ended a sentence" set to false, so the first word's punctuation is never looked at. `find_kp_breakpoints` starts from the same value for its line-length estimate.

Found with an automated differential tester that compares uutils with GNU 9.12.

I have a patch with tests and can open a PR.

### Reproduction steps

```sh
export LC_ALL=C
printf 'Hello.\nWorld.\n' | fmt
# GNU 9.12: Hello.  World.
# uutils:   Hello. World.
```
