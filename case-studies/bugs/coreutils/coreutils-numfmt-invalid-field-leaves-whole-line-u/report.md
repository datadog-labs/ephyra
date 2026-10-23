**Title:** numfmt: with `--invalid=fail/warn/ignore`, one invalid field leaves the whole line unconverted

### Describe the bug

When `--invalid` is `fail`, `warn` or `ignore` and one selected field on a line is not a number, uutils numfmt prints the input line unchanged: the valid fields on that line are not converted, and only the first invalid field gets a diagnostic. GNU numfmt converts every valid field, passes each invalid field through, and prints one diagnostic per invalid field.

The GNU manual (numfmt invocation) says:

> With a MODE of 'fail', print a warning for _each_ conversion error, and exit with status 2. With a MODE of 'warn', exit with status 0, even in the presence of conversion errors, and with a MODE of 'ignore' do not even print diagnostics.

```sh
printf 'x 1000\n' | numfmt --invalid=ignore --to=si --field=-
# GNU 9.12: x 1.0k
# uutils:   x 1000

printf 'x 1000 y 2000\n' | numfmt --invalid=fail --to=si --field=-
# GNU 9.12: x 1.0k y 2.0k
#           numfmt: invalid number: 'x'
#           numfmt: invalid number: 'y'      (exit 2)
# uutils:   x 1000 y 2000
#           numfmt: invalid number: 'x'      (exit 2)
```

Exit statuses already match; only stdout and the number of diagnostics differ.

**Versions.** uutils main eee56033d28bf7ef5cc816286b048779d57fb803 (2026-10-21) and the 0.12.0 release binary, macOS 26 aarch64 (the loop found it on Linux x86_64), `LC_ALL=C`, compared with GNU coreutils 9.12.

**Where.** `format_and_write` in `src/uu/numfmt/src/numfmt.rs`: in the non-abort modes, the field writers in `format.rs` stop at the first field that fails, and `format_and_write` then writes the original line ("On error, echo the original line unchanged").

Related: #15312 / PR #15327 (per-field rejection of scientific notation), a different case.

Found with an automated differential tester that compares uutils with GNU 9.12.

I have a patch with tests and can open a PR.

### Reproduction steps

```sh
export LC_ALL=C
printf 'x 1000\n' | numfmt --invalid=ignore --to=si --field=-
# GNU 9.12: x 1.0k
# uutils:   x 1000

printf 'x 1000 y 2000\n' | numfmt --invalid=fail --to=si --field=-; echo "exit=$?"
# GNU 9.12: x 1.0k y 2.0k, two "invalid number" lines ('x' and 'y'), exit=2
# uutils:   x 1000 y 2000, one "invalid number" line ('x'), exit=2
```
