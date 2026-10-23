# Unexpanded `~01` loses its leading zero and prints as `~1`

Posted as https://github.com/reubeno/brush/issues/1592 on 2026-10-23. The text below is the issue as posted.

---

### brush version

brush 0.4.0 (git:b562e058), built from main at b562e0586b366d0558b27019276e591856253947 (2026-10-22).

### bash version (for comparison)

GNU bash 5.3.20(1)-release

### Platform / OS

macOS

### Steps to reproduce

```sh
echo ~01
```

### Actual behavior (brush)

```sh
echo ~01
# brush: ~1
```

### Expected behavior

```sh
echo ~01
# bash 5.3: ~01
```

The directory stack has no entry 1, so the tilde-prefix should be left unchanged. brush leaves it unexpanded but rebuilds the text from the parsed number. The same happens with `~+01`, `~-007`, and in assignments (`D=~+02`). Indices that do exist expand correctly.

### Additional context

Found by an automated differential tester (brush vs bash 5.3); this report was drafted with AI assistance.
