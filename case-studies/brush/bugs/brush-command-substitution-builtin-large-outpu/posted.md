# Command substitution hangs when a builtin writes more than 64 KiB after a nested `$( )`

Posted as https://github.com/reubeno/brush/issues/1576 on 2026-10-23. The text below is the issue as posted.

---

### brush version

brush 0.4.0 (git:b562e058), built from main at b562e0586b366d0558b27019276e591856253947 (2026-10-22).

### bash version (for comparison)

GNU bash 5.3.20(1)-release

### Platform / OS

macOS

### Steps to reproduce

```sh
s=x; for i in {1..17}; do s=$s$s; done
for i in {1..20}; do v=$(echo "$(true)$s"); done; echo ok
```

`s` is 128 KiB long, more than a pipe buffer.

### Actual behavior (brush)

```sh
s=x; for i in {1..17}; do s=$s$s; done
for i in {1..20}; do v=$(echo "$(true)$s"); done; echo ok
# brush: (no output; never exits)
```

With `brush -c`, main hung in 10 of 10 runs and the 0.4.0 release in 8 of 10; run as a script file, main hung less often (1 of 3). A single `v=$(echo "$(true)$s")` hangs only sometimes, so the PoC repeats it. The hung process uses no CPU. Without the nested `$(true)` the loop finishes.

### Expected behavior

```sh
s=x; for i in {1..17}; do s=$s$s; done
for i in {1..20}; do v=$(echo "$(true)$s"); done; echo ok
# bash 5.3: ok
```

### Additional context

Found by an automated differential tester (brush vs bash 5.3); this report was drafted with AI assistance.
