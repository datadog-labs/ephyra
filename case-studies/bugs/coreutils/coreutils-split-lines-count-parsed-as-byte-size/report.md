**Title:** split: the `-l` count is parsed like a byte SIZE (`-l 010` means 8 lines, `-l 1k` is accepted)

### Describe the bug

uutils `split` parses the `-l` line count, and the obsolete `-NUMBER` form, with the same parser as the byte sizes of `-b` and `-C`. A leading `0` therefore makes the count octal (`-l 010` puts 8 lines in each file, not 10), and unit suffixes and hex are accepted (`-l 1k`, `-l 0x10`). GNU coreutils 9.12 reads 10 lines for `010` and rejects the other two with "invalid number of lines".

The GNU manual (split invocation) defines:

> `-l LINES`, `--lines=LINES`: Put LINES lines of INPUT into each output file.

The multiplicative suffixes are listed only under `-b SIZE` ("SIZE may be, or may be an integer optionally followed by, one of the following multiplicative suffixes"), and `-C SIZE` refers back to it. GNU coreutils 9.12:

```sh
seq 20 | split -l 010 && wc -l x*
# GNU 9.12: 10 xaa / 10 xab
# uutils:   8 xaa / 8 xab / 4 xac

seq 20 | split -l 1k; echo "exit=$?"
# GNU 9.12: split: invalid number of lines: '1k', exit=1
# uutils:   exit=0, writes xaa
```

`split -010` behaves like `-l 010`, and `split -l 0x10` writes 16-line files where GNU fails.

**Versions.** uutils main 342b6f7bd10adee39809516f56d86425422cc9ec (2026-10-22) and the 0.12.0 release binary, macOS 26 aarch64, `LC_ALL=C`, compared with GNU coreutils 9.12.

**Where.** `Strategy::from` in `src/uu/split/src/strategy.rs` parses `-l` and the obsolete `-NUMBER` with `parse_size_u64_max`, the function used for `-b` and `-C`.

Related: #15551 / PR #15564 (`od -w` accepts a size suffix), the same kind of problem in another utility.

Found with an automated differential tester that compares uutils with GNU 9.12. I reproduced it by hand on main and on 0.12.0.

### Reproduction steps

```sh
export LC_ALL=C
cd "$(mktemp -d)"

seq 20 | split -l 010; wc -l x*
# GNU 9.12: two files of 10 lines
# uutils:   three files of 8, 8 and 4 lines
```

I have a patch with tests and can open a PR.
