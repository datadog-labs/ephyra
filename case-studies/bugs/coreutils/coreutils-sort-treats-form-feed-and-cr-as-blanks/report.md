**Title:** sort: `-b` and `-M` still skip leading form feed and carriage return (follow-up to #14119)

### Describe the bug

#14119 (closed by PR #15436) was about uutils sort treating `\f` and `\r` as blanks. After #15436, `-n`, `-h`, `-d` and default field splitting match GNU. Two places still skip a leading `\f` or `\r`: the leading-blank trimming of `-b` (global or `-k N b`) and the month parser of `-M`. In the C locale a blank is only a space or a tab.

The GNU manual (sort invocation) says:

> `-b`: Ignore leading blanks when finding sort keys in each line. By default a blank is a space or a tab, but the LC_CTYPE locale can change this.
>
> `-M`: An initial string, consisting of any amount of blanks, followed by a month name abbreviation, is folded to UPPER case and compared in the order 'JAN' < 'FEB' < ... < 'DEC'. Invalid names compare low to valid names. ... By default a blank is a space or a tab

```sh
printf 'a\n\fb\n' | sort -k1b | od -An -c
# GNU 9.12:  \f   b  \n   a  \n      (the key keeps its \f, and \f < a)
# uutils:     a  \n  \f   b  \n      (the \f was skipped, so "b" > "a")

printf '\rFEB\nJAN\n' | sort -M | od -An -c
# GNU 9.12:  \r   F   E   B  \n   J   A   N  \n      ("\rFEB" is not a month, so it sorts low)
# uutils:     J   A   N  \n  \r   F   E   B  \n
```

`\f` and `\r` behave the same in both cases.

**Versions.** uutils main eee56033d28bf7ef5cc816286b048779d57fb803 (2026-10-21, includes #15436) and the 0.12.0 release binary, macOS 26 aarch64 (the loop found it on Linux x86_64), `LC_ALL=C`, compared with GNU coreutils 9.12.

**Where.** `src/uu/sort/src/sort.rs`: the `-b` key trimming uses `is_ascii_whitespace` (in the key-selection code around `position(|c| !c.is_ascii_whitespace())`), and `month_parse` starts with `line.trim_ascii_start()`.

Related: #14119 and PR #15436 cover `-n`, `-h`, `-d` and field splitting.

Found with an automated differential tester that compares uutils with GNU 9.12.

### Reproduction steps

```sh
export LC_ALL=C
printf 'a\n\fb\n' | sort -k1b | od -An -c
# GNU 9.12:  \f   b  \n   a  \n
# uutils:     a  \n  \f   b  \n

printf '\rFEB\nJAN\n' | sort -M | od -An -c
# GNU 9.12:  \r   F   E   B  \n   J   A   N  \n
# uutils:     J   A   N  \n  \r   F   E   B  \n
```
