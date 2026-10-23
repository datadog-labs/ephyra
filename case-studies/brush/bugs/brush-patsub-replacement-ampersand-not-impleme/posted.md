# `${var/pattern/string}`: unquoted `&` in the replacement is not replaced by the match (patsub_replacement is on but not implemented)

Posted as https://github.com/reubeno/brush/issues/1586 on 2026-10-23. The text below is the issue as posted.

---

### brush version

brush 0.4.0 (git:b562e058), built from main at b562e0586b366d0558b27019276e591856253947 (2026-10-22).

### bash version (for comparison)

GNU bash, version 5.3.20(1)-release (x86_64-pc-linux-gnu)

### Platform / OS

macOS

### Steps to reproduce

```bash
v=abc; r='<&>'; echo "${v/b/<&>}" "${v/b/$r}"
# bash:  a<b>c a<b>c
# brush: a<&>c a<&>c
```

### Actual behavior (brush)

```bash
v=abc; echo "${v/b/<&>}" "${v//?/[&]}"
# brush: a<&>c [&][&][&]
shopt patsub_replacement
# brush: patsub_replacement  	on
```

brush reports the option as on, but it inserts a literal `&`.

### Expected behavior

```bash
v=abc; echo "${v/b/<&>}" "${v//?/[&]}"
# bash: a<b>c [a][b][c]
shopt patsub_replacement
# bash: patsub_replacement  	on
```

bash manual, Parameter Expansion, `${parameter/pattern/string}`:
"If the patsub_replacement shell option is enabled using shopt, any unquoted instances of & in string are
replaced with the matching portion of pattern."
And under `shopt`: "patsub_replacement ... This option is enabled by default."

An escaped or quoted `&` (`\&`, `'&'`) already gives a literal `&` in brush, as it should.

### Additional context

Found by an automated differential tester (brush vs bash 5.3); this report was drafted with AI assistance.
