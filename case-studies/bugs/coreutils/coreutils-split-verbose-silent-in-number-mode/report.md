**Title:** split: `--verbose` prints nothing with `-n`/`--number`

### Describe the bug

`split --verbose` prints `creating file 'NAME'` before each output file with `-b`, `-l` and `-C`, but prints nothing with any `-n` mode (`N`, `l/N`, `r/N`, with or without `-e`). The files themselves are written correctly.

The GNU manual (split invocation) says:

> `--verbose`: Write a diagnostic just before each output file is opened.

```sh
printf 'a\nb\n' | split -n 2 --verbose
# GNU 9.12: creating file 'xaa'
#           creating file 'xab'
# uutils:   (nothing)

printf 'a\nb\n' | split -n l/2 --verbose
# GNU 9.12: creating file 'xaa'
#           creating file 'xab'
# uutils:   (nothing)
```

**Versions.** uutils main eee56033d28bf7ef5cc816286b048779d57fb803 (2026-10-21) and the 0.12.0 release binary, macOS 26 aarch64 (the loop found it on Linux x86_64), `LC_ALL=C`, compared with GNU coreutils 9.12.

**Where.** `src/uu/split/src/split.rs`: the `-n` modes open their files through `OutFiles` (`ManageOutFiles::init` and `instantiate_writer`), which never call `print_creating_file`.

Related: listed as one row (`split -n2 --verb`) in the bulk issue #15254.

Found with an automated differential tester that compares uutils with GNU 9.12.

I have a patch with tests and can open a PR.

### Reproduction steps

```sh
export LC_ALL=C
cd "$(mktemp -d)"
printf 'a\nb\n' | split -n 2 --verbose
# GNU 9.12: creating file 'xaa' / creating file 'xab'
# uutils:   no output
```
