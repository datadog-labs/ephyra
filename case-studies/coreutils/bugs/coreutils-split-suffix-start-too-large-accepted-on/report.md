**Title:** split: a suffix start value too large for the suffix length is accepted when input is empty (regression on main)

### Describe the bug

With `--numeric-suffixes=FROM` or `--hex-suffixes=FROM`, the suffix length is fixed, and a FROM that does not fit is an error ("numerical suffix start value is too large for the suffix length"). On current main this check is skipped when the input is empty: split exits 0 with no message. With input it still fails. GNU 9.12 and uutils 0.12.0 reject it for any input.

The GNU manual (split invocation) says:

> FROM is supported with the long form option, and is used to either set the initial suffix for a single run, or to set the suffix offset for independently split inputs, and consequently the auto suffix length expansion described above is disabled. Therefore you may also want to use option -a to allow suffixes beyond 99.

```sh
split --numeric-suffixes=100 </dev/null
# GNU 9.12:     split: numerical suffix start value is too large for the suffix length   (exit 1)
# uutils 0.12.0: the same error line                                                    (exit 1)
# uutils main:  (nothing)                                                              (exit 0)

printf 'a\n' | split --numeric-suffixes=100
# GNU 9.12 and uutils main: the same error, exit 1
```

**Versions.** uutils main eee56033d28bf7ef5cc816286b048779d57fb803 (2026-10-21), macOS 26 aarch64 (the loop found it on Linux x86_64), `LC_ALL=C`, compared with GNU coreutils 9.12. The 0.12.0 release is not affected.

**Where.** 1c28e6d99 (PR #14973, "split: avoid opening output for empty input") added an early return in `split()` (`src/uu/split/src/split.rs`, "Fixed-size modes only open an output when there is data to split"). The start-value check only runs in `FilenameIterator::new` when the first writer is created, so with empty input it never runs.

Found with an automated differential tester that compares uutils with GNU 9.12.

I have a patch with tests and can open a PR.

### Reproduction steps

```sh
export LC_ALL=C
cd "$(mktemp -d)"
split --numeric-suffixes=100 </dev/null; echo "exit=$?"
# GNU 9.12: "split: numerical suffix start value is too large for the suffix length", exit=1
# uutils:   exit=0

split -a 1 --hex-suffixes=15 </dev/null; echo "exit=$?"
# GNU 9.12: same error, exit=1
# uutils:   exit=0
```
