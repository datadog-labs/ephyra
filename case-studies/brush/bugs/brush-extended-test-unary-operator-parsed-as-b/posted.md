# `[[ -z == x ]]` is accepted as a string comparison; bash reads `-z` as the unary operator and rejects it

Posted as https://github.com/reubeno/brush/issues/1587 on 2026-10-23. The text below is the issue as posted.

---

### brush version

brush 0.4.0 (git:b562e058), built from main at b562e0586b366d0558b27019276e591856253947 (2026-10-22).

### bash version (for comparison)

GNU bash 5.3.20(1)-release

### Platform / OS

macOS

### Steps to reproduce

```sh
[[ -z == ]]; echo "rc=$?"
[[ -z == x ]]; echo "rc=$?"
```

Run each line on its own (for example with `-c`), since the second one is a syntax error in bash.

### Actual behavior (brush)

```sh
[[ -z == ]]; echo "rc=$?"
# brush: error: -c: syntax error at line 1 col 12   (exit 2)
[[ -z == x ]]; echo "rc=$?"
# brush: rc=1
```

### Expected behavior

```sh
[[ -z == ]]; echo "rc=$?"
# bash 5.3: rc=1
[[ -z == x ]]; echo "rc=$?"
# bash 5.3: syntax error in conditional expression: unexpected token `x'   (exit 2)
```

In bash an unquoted `-z` is the unary operator, and it takes the next word (`==`) as its operand. brush reads `-z` as a plain word and `==` as the binary operator. `[[ -f -nt x ]]` and `[[ -n || x ]]` are syntax errors in bash too, but brush evaluates them. A quoted `'-z'` is a plain string in both shells.

### Additional context

Found by an automated differential tester (brush vs bash 5.3); this report was drafted with AI assistance.
