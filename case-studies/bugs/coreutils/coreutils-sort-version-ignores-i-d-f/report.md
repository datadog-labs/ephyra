**Title:** sort: `-V` ignores `-d` and `-i` (follow-up to #15463)

### Describe the bug

#15463 reports that `-V` ignores `-f`, and open PR #15468 fixes that part. `-d` (`--dictionary-order`) and `-i` (`--ignore-nonprinting`) are dropped the same way with `-V`, globally and per key: uutils compares the raw key.

The GNU manual (sort invocation) says:

> `-d`: Sort in "phone directory" order: ignore all characters except letters, digits and blanks when sorting.
>
> `-V`: Sort by version name and number. It behaves like a standard sort, except that each sequence of decimal digits is treated numerically as an index/version number.

```sh
printf 'b\n_a\n' | sort -dV
# GNU 9.12: _a / b          (keys "a" and "b")
# uutils:   b / _a

printf '0\n\t\n' | sort -c -iV
# GNU 9.12: sort: -:2: disorder: <TAB>      (exit 1; the key of line 2 is empty)
# uutils:   (nothing)                       (exit 0)
```

`-k1dV` gives the same result as `-dV`.

**Versions.** uutils main eee56033d28bf7ef5cc816286b048779d57fb803 (2026-10-21) and the 0.12.0 release binary, macOS 26 aarch64 (the loop found it on Linux x86_64), `LC_ALL=C`, compared with GNU coreutils 9.12.

**Where.** `compare_by` in `src/uu/sort/src/sort.rs`: `SortMode::Version => version_cmp(a_str, b_str)` passes the unfiltered key, while the default arm applies `dictionary_order`, `ignore_non_printing` and `ignore_case`.

Related: #15463 and PR #15468 (`-f` with `-V`).

Found with an automated differential tester that compares uutils with GNU 9.12.

### Reproduction steps

```sh
export LC_ALL=C
printf 'b\n_a\n' | sort -dV
# GNU 9.12: _a, b
# uutils:   b, _a

printf '0\n\t\n' | sort -c -iV; echo "exit=$?"
# GNU 9.12: "sort: -:2: disorder: " followed by a tab, exit=1
# uutils:   no output, exit=0
```
