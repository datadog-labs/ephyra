**Title:** printf: `%.*s` accepts a `*` precision above 2147483647 instead of failing with "invalid precision"

### Describe the bug

When `%.*s` takes its precision from an argument larger than 2147483647 (INT_MAX), uutils `printf` prints the string and exits 0. GNU printf reports `invalid precision`, prints nothing for that directive and exits 1. uutils already rejects the same value for `%.*d` and `%.*f`, with the same message as GNU; only `%s` skips the check.

The GNU manual (printf invocation) says:

> `printf` prints the FORMAT string, interpreting `%` directives and `\` escapes to format numeric and string arguments in a way that is mostly similar to the C `printf` function

In C a `*` precision is an `int`. GNU coreutils 9.12:

```sh
env printf '%.*s|\n' 2147483648 x; echo "exit=$?"
# GNU 9.12: printf: invalid precision: '2147483648', exit=1
# uutils:   x|, exit=0

env printf '%.*d|\n' 2147483648 1; echo "exit=$?"
# GNU 9.12: printf: invalid precision: '2147483648', exit=1
# uutils:   printf: invalid precision: '2147483648', exit=1
```

uutils also goes on with the rest of the format (`env printf '%.*s %s\n' 2147483648 a b` prints `a b`), and with `9223372036854775807` it still prints the string and exits 0. A precision of 2147483647 is accepted by both, and so is a literal precision in the format (`%.4294967296s`); only a `*` precision is limited.

**Versions.** uutils main 342b6f7bd10adee39809516f56d86425422cc9ec (2026-10-22) and the 0.12.0 release binary, macOS 26 aarch64, `LC_ALL=C`, compared with GNU coreutils 9.12.

**Where.** The `Spec::String` arm in `src/uucore/src/lib/features/format/spec.rs` resolves the `*` precision without the `check_precision` call that the integer and float arms make.

Related: #13221 and #12708 (precision handling in numeric conversions), PR #13883 (`%s` field width). None covers this case.

Found with an automated differential tester that compares uutils with GNU 9.12. I reproduced it by hand on main and on 0.12.0.

### Reproduction steps

```sh
export LC_ALL=C

env printf '%.*s|\n' 2147483648 x; echo "exit=$?"
# GNU 9.12: "printf: invalid precision: '2147483648'" on stderr, exit=1
# uutils:   x|, exit=0
```

I have a patch with tests and can open a PR.
