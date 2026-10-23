**Title:** expand: `--tabs=2,/2` and `--tabs=4,+2` are rejected with "tab sizes must be ascending"

### Describe the bug

A trailing `/N` or `+N` in a `--tabs` list is a tab *size* for the remaining positions, not a tab position. uutils expand still checks it against the previous tab stop, so a size that is not larger than the last stop is rejected.

The GNU manual (expand invocation) says:

> As a GNU extension the last tab specified can be prefixed with a '/' to indicate a tab size to use for remaining positions. For example, --tabs=2,4,/8 will set tab stops at position 2 and 4, and every multiple of 8 after that.
>
> Also the last tab specified can be prefixed with a '+' to indicate a tab size to use for remaining positions, offset from the final explicitly specified tab stop.

```sh
printf 'a\tb\tc\n' | expand --tabs=2,/2
# GNU 9.12: "a b c"                                (stops at 2, 4, 6, ...)
# uutils:   expand: tab sizes must be ascending    (exit 1)

printf '\ta\tb\n' | expand --tabs=4,+2
# GNU 9.12: "    a b"                              (stops at 4, 6, 8, ...)
# uutils:   expand: tab sizes must be ascending    (exit 1)
```

uutils accepts the same lists when N happens to be larger than the last stop (`--tabs=2,/3`), and `unexpand -a --tabs=2,/2` accepts them too.

**Versions.** uutils main eee56033d28bf7ef5cc816286b048779d57fb803 (2026-10-21) and the 0.12.0 release binary, macOS 26 aarch64 (the loop found it on Linux x86_64), `LC_ALL=C`, compared with GNU coreutils 9.12.

**Where.** `tabstops_parse` in `src/uu/expand/src/expand.rs` runs the "must be ascending" check on the number after `/` or `+` as well.

Related: #4260 and #3612 added the `/N` and `+N` forms.

Found with an automated differential tester that compares uutils with GNU 9.12.

I have a patch with tests and can open a PR.

### Reproduction steps

```sh
export LC_ALL=C
printf 'a\tb\tc\n' | expand --tabs=2,/2; echo "exit=$?"
# GNU 9.12: "a b c", exit=0
# uutils:   "expand: tab sizes must be ascending", exit=1

printf '\ta\tb\n' | expand --tabs=4,+2; echo "exit=$?"
# GNU 9.12: "    a b", exit=0
# uutils:   "expand: tab sizes must be ascending", exit=1
```
