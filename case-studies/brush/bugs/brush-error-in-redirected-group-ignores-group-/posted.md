# Error inside a compound command with `2>` ignores that redirect

Posted as https://github.com/reubeno/brush/issues/1596 on 2026-10-23. The text below is the issue as posted.

---

### brush version

brush 0.4.0 (git:b562e058), built from main at b562e0586b366d0558b27019276e591856253947 (2026-10-22).

### bash version (for comparison)

GNU bash 5.3.20(1)-release

### Platform / OS

macOS

### Steps to reproduce

```sh
readonly r=1; { r=2; } 2>/dev/null
```

### Actual behavior (brush)

```sh
readonly r=1; { r=2; } 2>/dev/null
# brush: error: cannot mutate readonly variable      (on the shell's stderr)
```

### Expected behavior

```sh
readonly r=1; { r=2; } 2>/dev/null
# bash 5.3: (no output)
```

The exit status is 1 in both shells. The same happens with other errors that brush reports as an `Err`, for example `${x?msg}` or `a[-5]=y`, and with other compound commands such as `if ...; fi 2>/dev/null`.

### Additional context

Found by an automated differential tester (brush vs bash 5.3); this report was drafted with AI assistance.
