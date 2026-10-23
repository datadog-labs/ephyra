# nullglob removes a word like `[a/b]` that is not a pattern (regression since #1153)

Posted as https://github.com/reubeno/brush/issues/1582 on 2026-10-23. The text below is the issue as posted.

---

### brush version

brush 0.4.0 (git:b562e058), built from main at b562e0586b366d0558b27019276e591856253947 (2026-10-22).

### bash version (for comparison)

GNU bash 5.3.20(1)-release

### Platform / OS

macOS

### Steps to reproduce

```sh
shopt -s nullglob; echo [a/b]
```

Run it in a directory with no file or directory named `[a`.

### Actual behavior (brush)

```sh
shopt -s nullglob; echo [a/b]
# brush: (empty line)
```

### Expected behavior

```sh
shopt -s nullglob; echo [a/b]
# bash 5.3: [a/b]
```

A bracket expression cannot contain `/`, so for bash `[a/b]` has no pattern characters and is kept as it is, even with nullglob.

### Additional context

Found by an automated differential tester (brush vs bash 5.3); this report was drafted with AI assistance.
