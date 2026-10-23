**Title:** sort: a key's own `r` is ignored when the key is the whole line (`-k1r`)

### Describe the bug

With a single key that starts at field 1 and has no end (`-k1r`, `-k1.1r`, `-f -k1fr`), uutils sort drops the key's `r` and sorts ascending. `-k1,1r` works. The same bug makes `sort -c -k1r` accept input in ascending order.

The GNU manual (sort invocation) says:

> A position in a sort field specified with -k may have any of the option letters 'MbdfghinRrV' appended to it, in which case no global ordering options are inherited by that particular field.

and `sort --help`: "OPTS is one or more single-letter ordering options [bdfgiMhnRrV], which override global ordering options for that key."

```sh
printf 'a\nb\n' | sort -k1r
# GNU 9.12: b / a
# uutils:   a / b

printf 'a\nb\n' | sort -c -k1r
# GNU 9.12: sort: -:2: disorder: b      (exit 1)
# uutils:   (nothing)                   (exit 0)
```

**Versions.** uutils main eee56033d28bf7ef5cc816286b048779d57fb803 (2026-10-21) and the 0.12.0 release binary, macOS 26 aarch64 (the loop found it on Linux x86_64), `LC_ALL=C`, compared with GNU coreutils 9.12.

**Where.** `GlobalSettings::can_use_fast_lexicographic` and `can_use_fast_ascii_insensitive` in `src/uu/sort/src/sort.rs` enable a whole-line fast path for one key that needs no selection, without checking the key's own `reverse`. The fast path in `compare_by` reverses only for the global `-r`. The fast paths came with #8833.

Found with an automated differential tester that compares uutils with GNU 9.12.


### Reproduction steps

```sh
export LC_ALL=C
printf 'a\nb\n' | sort -k1r
# GNU 9.12: b, a
# uutils:   a, b

printf 'a\nb\n' | sort -k1,1r      # both: b, a
```
