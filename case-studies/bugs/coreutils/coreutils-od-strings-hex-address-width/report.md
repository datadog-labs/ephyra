**Title:** od: `-S` with `-Ax` prints 7-digit hex offsets instead of 6

### Describe the bug

With `-S`/`--strings` and `-Ax`, uutils od zero-pads string offsets to 7 hex digits. GNU od pads them to 6, and so does uutils od's own normal dump with `-Ax`. Decimal (`-Ad`) and octal (`-Ao`) offsets already use 7 digits in both.

The GNU manual does not spell out the width; it describes `-A RADIX` as selecting "the base in which file offsets are printed". GNU 9.12 uses 6 hex digits for every output mode, `-S` included.

```sh
printf 'a\0' | od -Ax -S1
# GNU 9.12: 000000 a
# uutils:   0000000 a

printf 'a\0' | od -Ax -c | head -1
# GNU 9.12: 000000   a  \0
# uutils:   000000   a  \0       (same uutils build, normal dump: 6 digits)
```

**Versions.** uutils main eee56033d28bf7ef5cc816286b048779d57fb803 (2026-10-21) and the 0.12.0 release binary, macOS 26 aarch64 (the loop found it on Linux x86_64), `LC_ALL=C`, compared with GNU coreutils 9.12.

**Where.** `extract_strings_from_input` in `src/uu/od/src/od.rs` formats the offset itself with `{offset:07x}`, while `InputOffset::format_byte_offset` (`input_offset.rs`) uses `{:06x}` for hex.

Related: #11613, #11744, #13396 (offset *values* with `-S -N`), not the width.

Found with an automated differential tester that compares uutils with GNU 9.12.

I have a patch with tests and can open a PR.

### Reproduction steps

```sh
export LC_ALL=C
printf 'a\0' | od -Ax -S1
# GNU 9.12: 000000 a
# uutils:   0000000 a
```
