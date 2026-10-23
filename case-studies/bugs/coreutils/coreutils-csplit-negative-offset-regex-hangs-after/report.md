**Title:** csplit: hangs on a `/REGEX/-N` pattern after a line-number pattern that names the line a regex stopped at

### Describe the bug

`csplit` never finishes when a regex pattern with a negative offset (`/REGEX/-N` or `%REGEX%-N`) follows a line-number pattern `N`, and an earlier regex pattern already stopped at line N (so the line-number pattern gives an empty split). csplit prints the sizes of the first splits and then spins at 100% CPU with no further output.

The GNU manual (csplit invocation) says:

> `/regexp/[offset]`: Create an output file containing the current line up to (but not including) the next line of the input file that contains a match for regexp. [...] If it is given, the input up to (but not including) the matching line plus or minus offset is put into the output file, and the line after that begins the next section of input.

and, when nothing matches, "An error occurs if a pattern argument refers to a nonexistent line of the input file (e.g., if no remaining line matches a given regular expression)." Either way csplit ends. GNU coreutils 9.12:

```sh
seq 3 | csplit - /2/ 2 /3/-1
# GNU 9.12: 2 / 0 / 0 / 4                         (exit 0)
# uutils:   2 / 0, then hangs                      (killed by timeout)

seq 5 | csplit - /3/-1 2 /x/-1
# GNU 9.12: 2 / 0 / 8, csplit: '/x/-1': match not found   (exit 1)
# uutils:   2 / 0, then hangs

seq 6 | csplit - /3/ 4 /5/-1
# GNU 9.12: 4 / 2 / 0 / 6
# uutils:   4 / 2 / 0 / 6      (no hang when the line-number pattern is past the regex stop)
```

`%3%-1` in place of `/3/-1` hangs the same way. No options are needed.

**Versions.** uutils main 342b6f7bd10adee39809516f56d86425422cc9ec (2026-10-22) and the 0.12.0 release binary, macOS 26 aarch64, `LC_ALL=C`, compared with GNU coreutils 9.12.

**Where.** `SplitWriter::do_to_line` (`src/uu/csplit/src/csplit.rs`) leaves the input buffer in rewind mode, and the negative-offset branch of `SplitWriter::do_to_match` then keeps taking the first buffered line and putting it back.

Related: #15440 is about the same hand-over from `do_to_line` to `do_to_match` (a regex after a line-number pattern does not test the held line). Its open PR #15444 (head 0acf7a9f4) does not fix this hang, and with it applied a positive regex offset hangs too, which main handles:

```sh
seq 6 | csplit - %2%+1 3 /5/-1
# GNU 9.12:      0 / 2 / 6
# uutils main:   0 / 2 / 6
# with PR #15444: 0, then hangs
```

Found with an automated differential tester that compares uutils with GNU 9.12. I reproduced it by hand on main, on 0.12.0 and on a build of PR #15444.

### Reproduction steps

```sh
export LC_ALL=C
cd "$(mktemp -d)"

seq 3 | timeout 10 csplit - /2/ 2 /3/-1; echo "exit=$?"
# GNU 9.12: 2 / 0 / 0 / 4, exit=0
# uutils:   2 / 0, exit=124 (killed by timeout)
```

I have a patch with tests and can open a PR.
