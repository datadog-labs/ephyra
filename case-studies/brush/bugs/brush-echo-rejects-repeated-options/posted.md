# `echo -n -n hi` fails with a usage error, and `-e` wins over a later `-E`

Posted as https://github.com/reubeno/brush/issues/1588 on 2026-10-23. The text below is the issue as posted.

---

### brush version

brush 0.4.0 (git:b562e058), built from main at b562e0586b366d0558b27019276e591856253947 (2026-10-22).

### bash version (for comparison)

GNU bash 5.3.20(1)-release

### Platform / OS

macOS

### Steps to reproduce

```sh
echo -n -n hi; echo " rc=$?"
echo -e -E 'a\tb'
```

### Actual behavior (brush)

```sh
echo -n -n hi; echo " rc=$?"
# brush: error: the argument '-n' cannot be used multiple times
#        Usage: echo [OPTIONS] [ARGS]...
#         rc=2
echo -e -E 'a\tb'
# brush: a<TAB>b
```

### Expected behavior

```sh
echo -n -n hi; echo " rc=$?"
# bash 5.3: hi rc=0
echo -e -E 'a\tb'
# bash 5.3: a\tb
```

bash accepts a repeated option, and of `-e` and `-E` the last one wins (`echo -E -e 'a\tb'` prints a tab in both shells). `echo -nn` and `echo -e -e` fail the same way in brush.

### Additional context

Found by an automated differential tester (brush vs bash 5.3); this report was drafted with AI assistance.
