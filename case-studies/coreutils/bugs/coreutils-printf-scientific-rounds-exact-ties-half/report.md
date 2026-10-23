**Title:** printf: `%e` and `%g` round an exact halfway value up instead of to even (`%.0e` of 2.5 gives `3e+00`)

### Describe the bug

`printf` with `%e`, `%E` or `%g` rounds a value that lies exactly halfway between two outputs away from zero. GNU printf, like C `printf`, rounds such a tie to even. uutils' own `%f` already rounds the same ties to even, so `%.0f` and `%.0e` of 2.5 disagree in uutils. `seq -f` shows the same.

0.125, 2.5, 0.25 and 1.125 are exact in binary, so these are true ties and not decimal-to-binary effects.

The GNU manual (printf invocation) says:

> `printf` prints the FORMAT string, interpreting `%` directives and `\` escapes to format numeric and string arguments in a way that is mostly similar to the C `printf` function

and (Floating point) "it is typically portable to assume IEEE-754 behavior these days." GNU coreutils 9.12:

```sh
env printf '%.1e %.0e %.1g %.0f\n' 0.125 2.5 0.25 2.5
# GNU 9.12: 1.2e-01 2e+00 0.2 2
# uutils:   1.3e-01 3e+00 0.3 2

seq -f '%.0e' 2.5 2.5
# GNU 9.12: 2e+00
# uutils:   3e+00
```

Values that are not ties round the same in both (`%.0e` of 3.5 is `4e+00`), and `%.2e` of 1.125 shows it is not only a precision-0 problem (GNU `1.12e+00`, uutils `1.13e+00`). GNU gives the same output on Linux x86_64 and on macOS aarch64.

**Versions.** uutils main 342b6f7bd10adee39809516f56d86425422cc9ec (2026-10-22) and the 0.12.0 release binary, macOS 26 aarch64, `LC_ALL=C`, compared with GNU coreutils 9.12 (also seen against GNU 9.12 on Linux x86_64).

**Where.** `bd_to_string_exp_with_prec` in `src/uucore/src/lib/features/format/num_format.rs` rounds with `BigDecimal::with_prec`, which rounds halves up.

Related: #13147 (`%.1f` of 2.45 and 2.55, which are not exact in binary) is a different question. This report covers only exact binary ties, where uutils `%f` already agrees with GNU and `%e`/`%g` do not.

Found with an automated differential tester that compares uutils with GNU 9.12. I reproduced it by hand on main and on 0.12.0.

### Reproduction steps

```sh
export LC_ALL=C

env printf '%.0e %.0f\n' 2.5 2.5
# GNU 9.12: 2e+00 2
# uutils:   3e+00 2
```
