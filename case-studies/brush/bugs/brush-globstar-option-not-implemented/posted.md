# `shopt -s globstar` is accepted, but `**` does not match recursively

Posted as https://github.com/reubeno/brush/issues/1581 on 2026-10-23. The text below is the issue as posted.

---

### brush version

brush 0.4.0 (git:b562e058), built from main at b562e0586b366d0558b27019276e591856253947 (2026-10-22).

### bash version (for comparison)

GNU bash 5.3.20(1)-release

### Platform / OS

macOS

### Steps to reproduce

In an empty directory:

```sh
mkdir sub; touch top.txt sub/deep.txt
shopt -s globstar
echo **
echo **/*.txt
```

### Actual behavior (brush)

```sh
echo **
# brush: sub top.txt
echo **/*.txt
# brush: sub/deep.txt
```

### Expected behavior

```sh
echo **
# bash 5.3: sub sub/deep.txt top.txt
echo **/*.txt
# bash 5.3: sub/deep.txt top.txt
```

With globstar set, `**` matches all files and zero or more directories. In brush it behaves like `*`.

### Additional context

Found by an automated differential tester (brush vs bash 5.3); this report was drafted with AI assistance.
