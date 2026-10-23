# `~-` expands to an empty string when OLDPWD is not set (for example at startup)

Posted as https://github.com/reubeno/brush/issues/1591 on 2026-10-23. The text below is the issue as posted.

---

### brush version

brush 0.4.0 (git:b562e058), built from main at b562e0586b366d0558b27019276e591856253947 (2026-10-22).

### bash version (for comparison)

GNU bash 5.3.20(1)-release

### Platform / OS

macOS

### Steps to reproduce

```sh
env -u OLDPWD brush --norc --noprofile -c 'printf "<%s>\n" ~- ~-/x'
```

### Actual behavior (brush)

```sh
env -u OLDPWD brush --norc --noprofile -c 'printf "<%s>\n" ~- ~-/x'
# brush: <>
#        </x>
```

### Expected behavior

```sh
env -u OLDPWD bash --norc --noprofile -c 'printf "<%s>\n" ~- ~-/x'
# bash 5.3: <~->
#           <~-/x>
```

The bash manual says `~-` is replaced by the value of OLDPWD "if it is set". At startup, without OLDPWD in the environment, OLDPWD is declared but has no value, so `~-` should stay as it is. After `unset OLDPWD`, brush already prints `~-`.

### Additional context

Found by an automated differential tester (brush vs bash 5.3); this report was drafted with AI assistance.
