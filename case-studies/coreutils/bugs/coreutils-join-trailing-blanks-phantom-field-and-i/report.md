**Title:** join: with `-i`, every line of a matching group is printed with the first line's join field

### Describe the bug

With `-i`/`--ignore-case`, lines whose keys differ only in case form one matching group. uutils join prints the join field of the group's *first* file1 line on every output line of the group, so a line with key `b` comes out as `B`. The output then contains text that is not on that input line. GNU prints each line's own join field.

The GNU manual (join invocation) says:

> `-i`, `--ignore-case`: Ignore differences in case when comparing keys.

and that "each output line consists of the join field, the remaining fields from file1, then the remaining fields from file2". `-i` changes only the comparison, not what is printed.

```sh
printf 'B\n' > f; printf 'B\nb\n' | join -i - f
# GNU 9.12: B / b
# uutils:   B / B

printf 'b z\n' > f; printf 'B x\nb y\n' | join -i - f
# GNU 9.12: B x z / b y z
# uutils:   B x z / B y z
```

`-o 0` shows the same thing (`B / b` in GNU, `B / B` in uutils).

**Versions.** uutils main eee56033d28bf7ef5cc816286b048779d57fb803 (2026-10-21) and the 0.12.0 release binary, macOS 26 aarch64 (the loop found it on Linux x86_64), `LC_ALL=C`, compared with GNU coreutils 9.12.

**Where.** `State::combine` in `src/uu/join/src/join.rs` sets `let key = self.get_current_key();` once, before looping over the group's file1 lines, and prints that key for every `line1`. Without `-i`, all keys in a group are byte-equal, so this does not show.

Found with an automated differential tester that compares uutils with GNU 9.12.

I have a patch with tests and can open a PR.

### Reproduction steps

```sh
export LC_ALL=C
cd "$(mktemp -d)"
printf 'B\n' > f
printf 'B\nb\n' | join -i - f
# GNU 9.12: B, b
# uutils:   B, B
```
