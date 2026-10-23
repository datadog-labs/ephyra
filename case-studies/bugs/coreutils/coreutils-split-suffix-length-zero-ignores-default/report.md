**Title:** split: `-a 0` with `-n N` uses a 1-character suffix instead of the default length 2

### Describe the bug

`-a 0` should act as if `-a` was not given, so suffixes start at length 2. With `-n`, uutils split instead uses the smallest width that fits the chunk count, so `split -n 2 -a 0` writes `xa xb` instead of `xaa xab`. Without `-a 0` the same command writes `xaa xab`. The names differ whenever the chunk count fits in one suffix character (N ≤ 26, N ≤ 10 with `-d`, N ≤ 16 with `-x`).

The GNU manual (split invocation) says:

> `-a LENGTH`: Use suffixes of length LENGTH. If a LENGTH of 0 is specified, this is the same as if (any previous) -a was not specified, and thus enables the default behavior, which starts the suffix length at 2, and unless -n or --numeric-suffixes=FROM is specified, will auto increase the length by 2 as required.

```sh
split -n 2 -a 0 </dev/null; ls
# GNU 9.12: xaa xab
# uutils:   xa  xb

split -d -n 10 -a 0 </dev/null; ls
# GNU 9.12: x00 x01 ... x09
# uutils:   x0  x1  ... x9
```

**Versions.** uutils main eee56033d28bf7ef5cc816286b048779d57fb803 (2026-10-21) and the 0.12.0 release binary, macOS 26 aarch64 (the loop found it on Linux x86_64), `LC_ALL=C`, compared with GNU coreutils 9.12.

**Where.** The suffix setup in `src/uu/split/src/filenames.rs`: for `Strategy::Number`, an explicit `-a 0` is treated as "length 0" and grown only to `required_length`, instead of starting from the default length 2.

Found with an automated differential tester that compares uutils with GNU 9.12.

I have a patch with tests and can open a PR.

### Reproduction steps

```sh
export LC_ALL=C
cd "$(mktemp -d)"
split -n 2 -a 0 </dev/null; ls
# GNU 9.12: xaa xab
# uutils:   xa xb
```
