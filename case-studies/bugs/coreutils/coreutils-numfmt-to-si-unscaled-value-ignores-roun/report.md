**Title:** numfmt: `--to=si`/`--to=iec` ignores `--round` for values from 10 to 999 that get no suffix

### Describe the bug

With `--to=si`, `--to=iec` or `--to=iec-i`, a value of 10 or more that is printed without a suffix is rounded half-to-even instead of with the `--round` method (default `from-zero`). Separately, the "does it need a suffix" test is made on the unrounded value, so 999.9 under `--round=down` becomes `0.9k` where GNU prints `999`.

The GNU manual (numfmt invocation) says:

> `--round=METHOD`: When converting number representations, round the number according to METHOD, which can be 'up', 'down', 'from-zero' (the default), 'towards-zero', 'nearest'.

```sh
numfmt --to=si 12.1
# GNU 9.12: 13          (from-zero)
# uutils:   12

numfmt --to=si --round=down 999.9
# GNU 9.12: 999
# uutils:   0.9k
```

This is not the case fixed in #11938 / PR #12032. There, values below 10 (one decimal place at the scale step) were changed to round like GNU, and they still match: `numfmt --to=si 1.4` gives `1` and `0.6` gives `1` in both. GNU 9.12 gives the results recorded in #11938 for those inputs, so the two sets are consistent. GNU applies the method once the value has two or more integer digits: `--to=si 12.1` gives `13`, `--round=down 12.9` gives `12`, `99.1` gives `100`. uutils gives `12`, `13` and `99`.

**Versions.** uutils main eee56033d28bf7ef5cc816286b048779d57fb803 (2026-10-21) and the 0.12.0 release binary, macOS 26 aarch64 (the loop found it on Linux x86_64), `LC_ALL=C`, compared with GNU coreutils 9.12.

**Where.** `src/uu/numfmt/src/format.rs`: `transform_to` prints the no-suffix value with `format_float(i2, 0)`, which ignores the rounding method, and `consider_suffix` compares the unrounded value with `bases[1] - 1.0`.

Related: #11938 (closed by PR #12032).

Found with an automated differential tester that compares uutils with GNU 9.12.

I have a patch with tests and can open a PR.

### Reproduction steps

```sh
export LC_ALL=C
numfmt --to=si 12.1 99.1
# GNU 9.12: 13 / 100
# uutils:   12 / 99

numfmt --to=si --round=down 999.9
# GNU 9.12: 999
# uutils:   0.9k

numfmt --to=si 1.4 0.6      # values below 10, fixed by #12032: both print 1 / 1
```
