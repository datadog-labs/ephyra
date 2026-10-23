**Title:** seq: `-w` pads to the width of the increment, which is never printed

### Describe the bug

`seq -w` counts the increment's integer width (its sign and any leading zeros included) when choosing the padding width. The increment is never printed, so a negative step like `-1` or a zero-padded step like `007` adds leading zeros that no printed number needs.

The GNU manual (seq invocation) says:

> `-w`, `--equal-width`: Print all numbers with the same width, by padding with leading zeros.

```sh
seq -w 5 -1 3
# GNU 9.12: 5 / 4 / 3
# uutils:   05 / 04 / 03

seq -w 1 007 8
# GNU 9.12: 1 / 8
# uutils:   001 / 008
```

The increment's fractional digits still matter, and already match: `seq -w 1 .5 2` prints `1.0 1.5 2.0` in both.

**Versions.** uutils main eee56033d28bf7ef5cc816286b048779d57fb803 (2026-10-21) and the 0.12.0 release binary, macOS 26 aarch64 (the loop found it on Linux x86_64), `LC_ALL=C`, compared with GNU coreutils 9.12.

**Where.** `uumain` in `src/uu/seq/src/seq.rs` computes the padding as `first.num_integral_digits.max(increment.num_integral_digits).max(last.num_integral_digits)`.

Related: #14153 / PR #14180 (`-w` with scientific-notation operands), a different cause.

Found with an automated differential tester that compares uutils with GNU 9.12.

I have a patch with tests and can open a PR.

### Reproduction steps

```sh
export LC_ALL=C
seq -w 5 -1 3
# GNU 9.12: 5 4 3 (one per line)
# uutils:   05 04 03
```
