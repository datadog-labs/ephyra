**Title:** sort: `-h` ignores `-f`, so lowercase suffixes such as `m`, `g`, `e` are not folded

### Describe the bug

With `-h`, `-f` has no effect in uutils sort. GNU applies the case fold before the human-numeric compare, so `2m` is read as `2M` (mega) and `1e` as `1E` (exa). uutils treats the lowercase letter as "no suffix". The same happens with a per-key `f` (`-k2,2hf`).

The GNU manual (sort invocation) says:

> `-f`: Fold lowercase characters into the equivalent uppercase characters when comparing so that, for example, 'b' and 'B' sort as equal.
>
> `-h`: Sort numerically, first by numeric sign (negative, zero, or positive); then by SI suffix (either empty, or 'k' or 'K', or one of 'MGTPEZYRQ', in that order ...

```sh
printf '2m\n1K\n' | sort -h -f
# GNU 9.12: 1K / 2m
# uutils:   2m / 1K

printf 'a 1e\nb 1K\n' | sort -k2,2hf
# GNU 9.12: b 1K / a 1e
# uutils:   a 1e / b 1K
```

Without `-f` both print `2m` before `1K`, since a lowercase `m` is not an SI suffix.

**Versions.** uutils main eee56033d28bf7ef5cc816286b048779d57fb803 (2026-10-21) and the 0.12.0 release binary, macOS 26 aarch64 (the loop found it on Linux x86_64), `LC_ALL=C`, compared with GNU coreutils 9.12.

**Where.** The `SortMode::HumanNumeric` arm of `compare_by` in `src/uu/sort/src/sort.rs` calls `human_numeric_str_cmp` (`numeric_str_cmp.rs`) on the raw key; its `get_unit` only knows uppercase suffixes and `k`, and `ignore_case` is never applied on this path.

Related: #15463 / PR #15468 (`-f` with `-V`), the same kind of gap for version sort. #14232 (`-h` and the locale decimal point) is different.

Found with an automated differential tester that compares uutils with GNU 9.12.

### Reproduction steps

```sh
export LC_ALL=C
printf '2m\n1K\n' | sort -h -f
# GNU 9.12: 1K, 2m
# uutils:   2m, 1K
```
