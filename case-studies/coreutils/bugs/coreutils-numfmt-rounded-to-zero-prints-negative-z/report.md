**Title:** numfmt: a negative number that rounds to zero is printed as `-0`

### Describe the bug

When `--round`, `--to-unit` or a `--format` precision rounds a small negative number to zero, uutils `numfmt` prints a signed zero (`-0`, `-0.0`, or `-0000` with a zero-padded format). GNU coreutils 9.12 prints an unsigned zero.

The GNU manual (numfmt invocation) says only:

> `--round=METHOD`: When converting number representations, round the number according to METHOD, which can be `up`, `down`, `from-zero` (the default), `towards-zero`, `nearest`.

It says nothing about the sign of a zero result, so the reference here is GNU 9.12's behaviour:

```sh
printf -- '-0.3\n' | numfmt --round=up --format=%.0f
# GNU 9.12: 0
# uutils:   -0

printf -- '-3\n' | numfmt --to-unit=10 --round=nearest
# GNU 9.12: 0
# uutils:   -0

printf -- '-0.3\n' | numfmt --round=up --format=%05.0f
# GNU 9.12: 00000
# uutils:   -0000
```

`numfmt --round=nearest --format=%.1f -- -0.04` gives `0.0` in GNU and `-0.0` in uutils. An input of `-0` already prints `0` in both (uutils has `test_negative_zero` for that), and `numfmt --to=si -- -0.4` prints `-0` in both, so only a zero produced by rounding in the plain (no `--to`) path differs.

GNU gives the same output on Linux x86_64 (80-bit `long double`) and on macOS aarch64 (64-bit `long double`), so this is not a floating-point precision effect.

**Versions.** uutils main 342b6f7bd10adee39809516f56d86425422cc9ec (2026-10-22) and the 0.12.0 release binary, macOS 26 aarch64, `LC_ALL=C`, compared with GNU coreutils 9.12 (also seen against GNU 9.12 on Linux x86_64).

**Where.** `transform_to` in `src/uu/numfmt/src/format.rs` passes the result of `round_with_precision`, which can be `-0.0`, on to `format_float`, which prints the sign of a negative zero.

Related: PR #14392 (merged, `--round` with a `--format` precision of 0), #11664 (closed, sign placement with zero-padded `--format`), PR #4076 (merged, `-0` input). None covers this case.

Found with an automated differential tester that compares uutils with GNU 9.12. I reproduced it by hand on main and on 0.12.0.

### Reproduction steps

```sh
export LC_ALL=C

printf -- '-0.3\n' | numfmt --round=up --format=%.0f
# GNU 9.12: 0
# uutils:   -0
```

I have a patch with tests and can open a PR.
