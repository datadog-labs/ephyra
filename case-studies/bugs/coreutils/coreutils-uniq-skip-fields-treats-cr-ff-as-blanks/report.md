**Title:** uniq: `-f` treats carriage return and form feed as field separators

### Describe the bug

`uniq -f N` (`--skip-fields`) ends a field at `\r` and `\f`. Only blanks (space and tab) should separate fields, so `a\rb` is one field. Lines that GNU treats as duplicates are printed as different lines.

The GNU manual (uniq invocation) says for `-f N`:

> Fields are a sequence of blank characters followed by non-blank characters.

and GNU 9.12's `--help` says "A field is a run of blanks (usually spaces and/or TABs), then non-blank characters." GNU coreutils 9.12:

```sh
printf 'a\rb\nc\rd\n' | uniq -f1 | od -An -c
# GNU 9.12:  a  \r   b  \n
# uutils:    a  \r   b  \n   c  \r   d  \n

printf 'a\fb\nc\fd\n' | uniq -c -f1 | od -An -c
# GNU 9.12:  2 lines counted together (      2 a\fb)
# uutils:    two lines (      1 a\fb, then       1 c\fd)
```

Vertical tab already behaves the same in both (`printf 'a\vb\nc\vd\n' | uniq -f1` prints one line).

**Versions.** uutils main 342b6f7bd10adee39809516f56d86425422cc9ec (2026-10-22) and the 0.12.0 release binary, macOS 26 aarch64, `LC_ALL=C`, compared with GNU coreutils 9.12.

**Where.** `Uniq::skip_fields_offset()` in `src/uu/uniq/src/uniq.rs` finds field boundaries with `u8::is_ascii_whitespace()`, which also matches `\r` and `\f`.

Related: the same kind of problem in sort, #14119 and #15588.

Found with an automated differential tester that compares uutils with GNU 9.12. I reproduced it by hand on main and on 0.12.0.

### Reproduction steps

```sh
export LC_ALL=C

printf 'a\rb\nc\rd\n' | uniq -f1 | od -c
# GNU 9.12: one line, a \r b
# uutils:   two lines, a \r b and c \r d
```

I have a patch with tests and can open a PR.
