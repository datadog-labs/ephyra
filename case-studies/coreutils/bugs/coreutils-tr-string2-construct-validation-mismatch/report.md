**Title:** tr: `[c*]` accepted with `-d -s`, `[=c=]` accepted in string2 when translating, and a case class past the end of string1 rejected

### Describe the bug

uutils tr checks some string2 constructs against the wrong mode. Open PRs #14400 and #15009 cover the class-alignment cases (`-ds` with `[:lower:]`, non-case classes, `-c`). Three cases remain with both PRs applied:

1. With `-d -s` nothing is translated, but uutils accepts `[c*]` in string2. GNU allows `[c*]` only when translating.
2. When translating, uutils accepts `[=c=]` in string2. GNU allows it only with `-d -s`.
3. When translating, a `[:lower:]`/`[:upper:]` in string2 that starts past the end of string1 (so it is never used) is rejected by uutils and accepted by GNU.

The GNU manual (tr invocation, "Character arrays") says:

> The notation '[C*]' in STRING2 expands to as many copies of C as are needed to make ARRAY2 as long as ARRAY1.
>
> '[=C=]' ... These equivalence classes are allowed in STRING2 only when --delete (-d) and --squeeze-repeats -s are both given.

Case 3 is not covered by the manual text; the expected result is GNU 9.12's.

```sh
printf 'xaab\n' | tr -ds a '[b*]'
# GNU 9.12: tr: the [c*] construct may appear in string2 only when translating     (exit 1)
# uutils:   xb                                                                     (exit 0)

printf 'xaab\n' | tr a '[=b=]'
# GNU 9.12: tr: [=c=] expressions may not appear in string2 when translating       (exit 1)
# uutils:   xbbb                                                                   (exit 0)

printf 'xaAb\n' | tr a 'ab[:lower:]'
# GNU 9.12: xaAb                                                                   (exit 0)
# uutils:   tr: when translating, every 'upper'/'lower' in set2 must be matched by a 'upper'/'lower' in the same position in set1   (exit 1)
```

With PR #14400 applied, case 3 fails with `misaligned [:upper:] and/or [:lower:] construct` instead; with PR #15009 it is unchanged. Cases 1 and 2 are unchanged by both.

**Versions.** uutils main eee56033d28bf7ef5cc816286b048779d57fb803 (2026-10-21), the 0.12.0 release binary, and builds of PRs #14400 and #15009, macOS 26 aarch64 (the loop found it on Linux x86_64), `LC_ALL=C`, compared with GNU coreutils 9.12.

**Where.** `Sequence::solve_set_characters` in `src/uu/tr/src/operation.rs`: no check rejects `[=c=]` in set2 when translating or `[c*]` when not translating, and the upper/lower position check also runs for classes beyond set1's length.

Related: PR #14400, PR #15009.

Found with an automated differential tester that compares uutils with GNU 9.12.

### Reproduction steps

```sh
export LC_ALL=C
printf 'xaab\n' | tr -ds a '[b*]'; echo "exit=$?"
# GNU 9.12: "tr: the [c*] construct may appear in string2 only when translating", exit=1
# uutils:   xb, exit=0

printf 'xaab\n' | tr a '[=b=]'; echo "exit=$?"
# GNU 9.12: "tr: [=c=] expressions may not appear in string2 when translating", exit=1
# uutils:   xbbb, exit=0

printf 'xaAb\n' | tr a 'ab[:lower:]'; echo "exit=$?"
# GNU 9.12: xaAb, exit=0
# uutils:   "tr: when translating, every 'upper'/'lower' in set2 must be matched ...", exit=1
```
