# printf: a large `*` width aborts printf with "formatting width too large" and drops the rest of the output

Posted as https://github.com/reubeno/brush/issues/1590 on 2026-10-23. The text below is the issue as posted.

---

### brush version

brush 0.4.0 (git:b562e058), built from main at b562e0586b366d0558b27019276e591856253947 (2026-10-22).

### bash version (for comparison)

GNU bash 5.3.20(1)-release

### Platform / OS

macOS

### Steps to reproduce

```sh
printf '[%*s]\n' 99999999999 x; echo "rc=$?"
```

### Actual behavior (brush)

```sh
printf '[%*s]\n' 99999999999 x; echo "rc=$?"
# brush: error: printf: i/o error: formatting width too large
#        [rc=1
```

### Expected behavior

```sh
printf '[%*s]\n' 99999999999 x; echo "rc=$?"
# bash 5.3: bash: line 1: printf: 99999999999: Result too large
#           [x]
#           rc=1
```

bash reports the width, ignores it, and prints the rest of the output. brush stops after `[`.

### Additional context

Found by an automated differential tester (brush vs bash 5.3); this report was drafted with AI assistance.
