**Title:** csplit: with `--suppress-matched`, a line number one past the end of input is rejected as out of range

### Describe the bug

With `--suppress-matched`, a line-number pattern `N` splits before line N and drops line N. When the input has exactly N-1 lines there is no line N to drop, so the split is complete. GNU csplit accepts this and writes an empty last file. uutils csplit reports `line number out of range`, exits 1, and (without `-k`) deletes the file it already wrote.

The GNU manual (csplit invocation) says:

> `--suppress-matched`: Do not output lines matching the specified pattern. I.e., suppress the boundary line from the start of the second and subsequent splits.

```sh
printf 'a\n' | csplit --suppress-matched - 2
# GNU 9.12: 2 / 0, exit 0            (xx00 = "a", xx01 empty)
# uutils:   2
#           csplit: '2': line number out of range      (exit 1, no files left)

seq 3 | csplit -k --suppress-matched - 2 '{*}'
# GNU 9.12: 2 / 2 / 0, "csplit: '2': line number out of range on repetition 2"   (exit 1, files 1 / 3 / empty)
# uutils:   2 / 2,     "csplit: '2': line number out of range on repetition 1"   (exit 1, files 1 / 3)
```

Only this one case differs: without `--suppress-matched` both reject N = lines+1, and with it both reject N = lines+2.

**Versions.** uutils main eee56033d28bf7ef5cc816286b048779d57fb803 (2026-10-21) and the 0.12.0 release binary, macOS 26 aarch64 (the loop found it on Linux x86_64), `LC_ALL=C`, compared with GNU coreutils 9.12.

**Where.** `SplitWriter::do_to_line` in `src/uu/csplit/src/csplit.rs` returns `LineOutOfRange` unless it reads line N itself, so running out of input right after line N-1 is an error even when line N would have been suppressed.

Related: #6126 (`--suppress-matched` with a line number after a regex), #7286 / #8655 (final empty file). #15481 (merged 2026-10-19, suppressing lines after an earlier numeric pattern) is in the main build above and does not change this.

Found with an automated differential tester that compares uutils with GNU 9.12.

### Reproduction steps

```sh
export LC_ALL=C
cd "$(mktemp -d)"

printf 'a\n' | csplit --suppress-matched - 2; echo "exit=$?"; ls
# GNU 9.12: 2 / 0, exit=0, xx00 xx01
# uutils:   2, "csplit: '2': line number out of range", exit=1, no files
```
