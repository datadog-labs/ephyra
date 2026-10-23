# `+=` on a `declare -i` variable panics with "attempt to add with overflow" in builds with overflow checks

Posted as https://github.com/reubeno/brush/issues/1577 on 2026-10-23. The text below is the issue as posted.

---

### brush version

brush 0.4.0 (git:b562e058), built from main at b562e0586b366d0558b27019276e591856253947 (2026-10-22).

### bash version (for comparison)

GNU bash 5.3.20(1)-release

### Platform / OS

macOS

### Steps to reproduce

```sh
declare -i n=1; n+=9223372036854775807; echo "$n"
```

Run it with a debug build of main (`cargo build -p brush-shell`, then `target/debug/brush`).

### Actual behavior (brush)

```sh
declare -i n=1; n+=9223372036854775807; echo "$n"
# brush (debug build): thread 'main' (9106084) panicked at brush-core/src/variables.rs:276:45:
#                      attempt to add with overflow
#                      (exit status 101)
```

### Expected behavior

```sh
declare -i n=1; n+=9223372036854775807; echo "$n"
# bash 5.3: -9223372036854775808
```

The bash manual (Shell Arithmetic) says evaluation is done in fixed-width integers "with no check for overflow".

### Additional context

Found by an automated differential tester (brush vs bash 5.3); this report was drafted with AI assistance.
