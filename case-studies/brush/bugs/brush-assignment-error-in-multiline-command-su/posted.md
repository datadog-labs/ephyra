# An error on one line of a multi-line `$( )` does not end the command substitution

Posted as https://github.com/reubeno/brush/issues/1593 on 2026-10-23. The text below is the issue as posted.

---

### brush version

brush 0.4.0 (git:b562e058), built from main at b562e0586b366d0558b27019276e591856253947 (2026-10-22).

### bash version (for comparison)

GNU bash 5.3.20(1)-release

### Platform / OS

macOS

### Steps to reproduce

```sh
v=$(
x=$((1/0))
echo hi
)
echo "v=<$v> rc=$?"
```

### Actual behavior (brush)

```sh
v=$(
x=$((1/0))
echo hi
)
echo "v=<$v> rc=$?"
# brush: v=<hi> rc=0
```

### Expected behavior

```sh
v=$(
x=$((1/0))
echo hi
)
echo "v=<$v> rc=$?"
# bash 5.3: v=<> rc=1
```

When the two commands are on one line, `v=$(x=$((1/0)); echo hi)`, brush already prints `v=<> rc=1` like bash. The difference only shows when the substitution has more than one line. Other errors behave the same way, for example `a=(x)` then `a[-2]=y`, or assigning to a readonly variable.

### Additional context

Found by an automated differential tester (brush vs bash 5.3); this report was drafted with AI assistance.
