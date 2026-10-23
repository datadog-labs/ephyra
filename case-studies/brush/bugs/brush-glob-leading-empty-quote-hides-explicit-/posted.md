# Glob `"".*` (empty quotes before the leading dot) does not match dot files

Posted as https://github.com/reubeno/brush/issues/1584 on 2026-10-23. The text below is the issue as posted.

---

### brush version

brush 0.4.0 (git:b562e058), built from main at b562e0586b366d0558b27019276e591856253947 (2026-10-22).

### bash version (for comparison)

GNU bash 5.3.20(1)-release

### Platform / OS

macOS

### Steps to reproduce

```sh
touch .h; echo "".*
```

### Actual behavior (brush)

```sh
touch .h; echo "".*
# brush: .*
```

### Expected behavior

```sh
touch .h; echo "".*
# bash 5.3: .h
```

After quote removal the pattern is `.*`, so the leading dot is given explicitly and dot files should match (with `dotglob` off). Plain `.*` works in brush. The same happens with `''.*`, with an empty variable before the dot (`e=; echo $e"".*`), and after a slash (`sub/"".*`).

### Additional context

Found by an automated differential tester (brush vs bash 5.3); this report was drafted with AI assistance.
