# `break`/`continue` outside a loop, or with a count larger than the loop depth, skip the rest of the line or fail the function

Posted as https://github.com/reubeno/brush/issues/1595 on 2026-10-23. The text below is the issue as posted.

---

### brush version

brush 0.4.0 (git:b562e058), built from main at b562e0586b366d0558b27019276e591856253947 (2026-10-22).

### bash version (for comparison)

GNU bash 5.3.20(1)-release

### Platform / OS

macOS

### Steps to reproduce

```sh
f() { break; echo "f goes on"; }; f; echo "rc=$?"
for i in 1; do break 2; done; echo after
```

Run each line on its own (for example with `-c`).

### Actual behavior (brush)

```sh
f() { break; echo "f goes on"; }; f; echo "rc=$?"
# brush: error: not yet implemented: break or continue returned from function invocation
#        rc=99

for i in 1; do break 2; done; echo after
# brush: (no output)
```

### Expected behavior

```sh
f() { break; echo "f goes on"; }; f; echo "rc=$?"
# bash 5.3: bash: line 1: break: only meaningful in a `for', `while', or `until' loop
#           f goes on
#           rc=0

for i in 1; do break 2; done; echo after
# bash 5.3: after
```

A plain `break; echo after` at top level is also different: bash warns and prints `after`, brush prints nothing.

### Additional context

Found by an automated differential tester (brush vs bash 5.3); this report was drafted with AI assistance.
