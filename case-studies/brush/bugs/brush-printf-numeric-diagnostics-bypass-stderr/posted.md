# printf: "invalid number" message ignores `2>` redirection, and the exit status is 0

Posted as https://github.com/reubeno/brush/issues/1589 on 2026-10-23. The text below is the issue as posted.

---

### brush version

brush 0.4.0 (git:b562e058), built from main at b562e0586b366d0558b27019276e591856253947 (2026-10-22).

### bash version (for comparison)

GNU bash 5.3.20(1)-release

### Platform / OS

macOS

### Steps to reproduce

```sh
printf '%d\n' x 2>/dev/null; echo "rc=$?"
```

### Actual behavior (brush)

```sh
printf '%d\n' x 2>/dev/null; echo "rc=$?"
# brush: brush: 'x': expected a numeric value      (on the shell's stderr)
#        0
#        rc=0
```

### Expected behavior

```sh
printf '%d\n' x 2>/dev/null; echo "rc=$?"
# bash 5.3: 0
#           rc=1
```

Without the redirect, bash prints `printf: x: invalid number` on stderr. A redirect on an enclosing function or `{ ...; }` group is ignored by brush in the same way.

### Additional context

Found by an automated differential tester (brush vs bash 5.3); this report was drafted with AI assistance.
