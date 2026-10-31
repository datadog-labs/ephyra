# Bugs: brush

The bugs the case studies found in brush. Each folder has the report as
it was filed upstream, the reproduction, and where we had one, the fix and
a test. Status as of this export; the upstream link is authoritative.

| Bug | Status | Upstream |
|---|---|---|
| [`${v:=word}` expands to `word` instead of the new value of `v` when `v` has `declare -u/-l/-c`](brush-assign-default-expansion-ignores-variabl/) | open | [link](https://github.com/reubeno/brush/issues/1585) |
| [An assignment error inside a function does not abandon the rest of the calling line](brush-assignment-error-in-function-does-not-ab/) | open | [link](https://github.com/reubeno/brush/issues/1594) |
| [An error on one line of a multi-line `$( )` does not end the command substitution](brush-assignment-error-in-multiline-command-su/) | open | [link](https://github.com/reubeno/brush/issues/1593) |
| [`break`/`continue` outside a loop, or with a count larger than the loop depth, skip the rest of the line or fail the function](brush-break-continue-ignore-loop-nesting/) | open | [link](https://github.com/reubeno/brush/issues/1595) |
| [Command substitution hangs when a builtin writes more than 64 KiB after a nested `$( )`](brush-command-substitution-builtin-large-outpu/) | open | [link](https://github.com/reubeno/brush/issues/1576) |
| [Brace expansion is not applied to assignment arguments of `declare`/`local`/`export`](brush-declaration-builtin-assignment-args-skip/) | open | [link](https://github.com/reubeno/brush/issues/1569) |
| [`echo -n -n hi` fails with a usage error, and `-e` wins over a later `-E`](brush-echo-rejects-repeated-options/) | fixed | [link](https://github.com/reubeno/brush/issues/1588) |
| [Error inside a compound command with `2>` ignores that redirect](brush-error-in-redirected-group-ignores-group-/) | open | [link](https://github.com/reubeno/brush/issues/1596) |
| [`[[ -z == x ]]` is accepted as a string comparison; bash reads `-z` as the unary operator and rejects it](brush-extended-test-unary-operator-parsed-as-b/) | fixed | [link](https://github.com/reubeno/brush/issues/1587) |
| [declare -f indents lines inside multi-line strings and here-documents, changing the function](brush-function-display-indents-inside-multilin/) | open | [link](https://github.com/reubeno/brush/issues/1580) |
| [nullglob removes a word like `[a/b]` that is not a pattern (regression since #1153)](brush-glob-bracket-with-slash-treated-as-patte/) | open | [link](https://github.com/reubeno/brush/issues/1582) |
| [Glob `"".*` (empty quotes before the leading dot) does not match dot files](brush-glob-leading-empty-quote-hides-explicit-/) | fixed | [link](https://github.com/reubeno/brush/issues/1584) |
| [Pathname expansion: backslash from a variable is kept in a glob path component that has no glob characters](brush-glob-literal-component-keeps-expansion-b/) | fixed | [link](https://github.com/reubeno/brush/issues/1583) |
| [`shopt -s globstar` is accepted, but `**` does not match recursively](brush-globstar-option-not-implemented/) | open | [link](https://github.com/reubeno/brush/issues/1581) |
| [`+=` on a `declare -i` variable panics with "attempt to add with overflow" in builds with overflow checks](brush-integer-append-add-overflow/) | open | [link](https://github.com/reubeno/brush/issues/1577) |
| [Negative subscript on a sparse array counts from the number of elements, not from the highest index](brush-negative-subscript-sparse-array-uses-ele/) | fixed | [link](https://github.com/reubeno/brush/issues/1570) |
| [`${var/pattern/string}`: unquoted `&` in the replacement is not replaced by the match (patsub_replacement is on but not implemented)](brush-patsub-replacement-ampersand-not-impleme/) | open | [link](https://github.com/reubeno/brush/issues/1586) |
| [printf: "invalid number" message ignores `2>` redirection, and the exit status is 0](brush-printf-numeric-diagnostics-bypass-stderr/) | open | [link](https://github.com/reubeno/brush/issues/1589) |
| [printf: a large `*` width aborts printf with "formatting width too large" and drops the rest of the output](brush-printf-star-width-out-of-int-range-abort/) | open | [link](https://github.com/reubeno/brush/issues/1590) |
| [`"${a[@]+x}"` with an empty array gives one empty word instead of no words](brush-quoted-at-alternative-empty-array-yields/) | fixed | [link](https://github.com/reubeno/brush/issues/1568) |
| [read drops control characters from a pipe, file or here-string, and treats ^C/^D in the data as keystrokes](brush-read-drops-control-chars-from-non-tty-in/) | open | [link](https://github.com/reubeno/brush/issues/1579) |
| [A syntax error later in a script file or -c string stops every earlier command from running](brush-syntax-error-in-script-suppresses-earlie/) | open | [link](https://github.com/reubeno/brush/issues/1578) |
| [Unexpanded `~01` loses its leading zero and prints as `~1`](brush-tilde-dirstack-prefix-drops-leading-zero/) | open | [link](https://github.com/reubeno/brush/issues/1592) |
| [`~-` expands to an empty string when OLDPWD is not set (for example at startup)](brush-tilde-minus-unset-oldpwd-expands-empty/) | fixed | [link](https://github.com/reubeno/brush/issues/1591) |
