**Title:** csplit: a `/REGEX/` pattern after a line-number pattern never tests the line the split stopped at

### Describe the bug

After a line-number pattern `N`, line N is the current line. A following `/REGEX/` pattern should test it. uutils csplit starts searching at line N+1, so line N goes into the next file without being tested.

The GNU manual (csplit invocation) says:

> `n`: Create an output file containing the input up to but not including line n (a positive integer).
>
> `/regexp/[offset]`: Create an output file containing the current line up to (but not including) the next line of the input file that contains a match for regexp.

The only lines the manual excludes from later regexp searches are "lines within a negative offset of a regexp pattern". GNU coreutils 9.12 tests line N:

```sh
printf 'a\n1\n2\n' | csplit - 2 '/[0-9]/'
# GNU 9.12: 2 / 0 / 4
# uutils:   2 / 2 / 2      (line "1" matches /[0-9]/ but goes into xx01)

seq 4 | csplit - 3 /3/
# GNU 9.12: 4 / 0 / 4
# uutils:   4 / 4
#           csplit: '/3/': match not found      (exit 1)

seq 50 > 50.txt; csplit 50.txt 10 /10/ -k
# GNU 9.12: 18 / 0 / 123
# uutils:   18 / 123
#           csplit: '/10/': match not found     (exit 1)
```

So lines land in the wrong output file. When the only match is on line N, csplit fails with "match not found", and without `-k` it deletes the files it already wrote. No options are needed to hit it.

Upstream's tests already note the difference. In `tests/by-util/test_csplit.rs`, `test_line_num_range_with_up_to_match3` (the third command above) has the comment `// NOTE: output different than gnu's: the pattern /10/ is matched but should not`. GNU does match line 10 there and prints `18 0 123`.

**Versions.** uutils main 2e28cc3042006f5811e05040232b6154bb905465 and the 0.12.0 release binary, Linux x86_64, `LC_ALL=C`, compared with GNU coreutils 9.12 (and 9.4, which prints the same).

**Where.** `SplitWriter::do_to_line` stops at line N and leaves it buffered for the next pattern. `SplitWriter::do_to_match` then writes the buffered lines into the new split without matching them (`// NOTE: drain the buffer of input_iter, no match should be done within.`). That is right for lines an earlier regex already looked at, but not for a line held back by a line-number pattern.

Related, in the same function: #6126 (`--suppress-matched` with a line number that was already passed).

Found with an automated differential tester that compares uutils with GNU 9.12. I reproduced it by hand on main and on 0.12.0.

### Reproduction steps

```sh
export LC_ALL=C
cd "$(mktemp -d)"

printf 'a\n1\n2\n' | csplit - 2 '/[0-9]/'; echo "exit=$?"
# GNU 9.12: 2 / 0 / 4, exit=0   (xx00 = "a", xx01 = empty, xx02 = "1\n2")
# uutils:   2 / 2 / 2, exit=0   (xx00 = "a", xx01 = "1",   xx02 = "2")
rm -f xx*

seq 4 | csplit - 3 /3/; echo "exit=$?"
# GNU 9.12: 4 / 0 / 4, exit=0
# uutils:   4 / 4, "csplit: '/3/': match not found", exit=1
```
