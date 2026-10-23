# `${v:=word}` expands to `word` instead of the new value of `v` when `v` has `declare -u/-l/-c`

Posted as https://github.com/reubeno/brush/issues/1585 on 2026-10-23. The text below is the issue as posted.

---

### brush version

brush 0.4.0 (git:b562e058), built from main at b562e0586b366d0558b27019276e591856253947 (2026-10-22).

### bash version (for comparison)

GNU bash 5.3.20(1)-release

### Platform / OS

macOS

### Steps to reproduce

```sh
declare -u v; echo "${v:=abc} $v"
```

### Actual behavior (brush)

```sh
declare -u v; echo "${v:=abc} $v"
# brush: abc ABC
```

The variable gets the upper-cased value, but the expansion itself is the original word. The same happens with `${v=abc}`, with `declare -l` and `declare -c`, and with array elements (`declare -u -a a; echo "${a[1]:=q}"` prints `q`).

### Expected behavior

```sh
declare -u v; echo "${v:=abc} $v"
# bash 5.3: ABC ABC
```

The bash manual says for `${parameter:=word}`: "the expansion of word is assigned to parameter, and the result of the expansion is the final value of parameter."

### Additional context

Found by an automated differential tester (brush vs bash 5.3); this report was drafted with AI assistance.
