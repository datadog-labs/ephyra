# An assignment error inside a function does not abandon the rest of the calling line

Posted as https://github.com/reubeno/brush/issues/1594 on 2026-10-23. The text below is the issue as posted.

---

### brush version

brush 0.4.0 (git:b562e058), built from main at b562e0586b366d0558b27019276e591856253947 (2026-10-22).

### bash version (for comparison)

GNU bash 5.3.20(1)-release

### Platform / OS

macOS

### Steps to reproduce

```sh
f() { a=(x); a[-5]=y; }
f; echo "same line"
echo "next line"
```

### Actual behavior (brush)

```sh
brush --norc --noprofile -c 'f() { a=(x); a[-5]=y; }
f; echo "same line"
echo "next line"' 2>/dev/null
# brush: same line
#        next line
```

### Expected behavior

```sh
bash --norc --noprofile -c 'f() { a=(x); a[-5]=y; }
f; echo "same line"
echo "next line"' 2>/dev/null
# bash 5.3: next line
```

bash treats the error as if it happened in the calling command, so it drops the rest of that line (`echo "same line"`) and goes on with the next line. brush already does this when the same assignment is written at top level (`a=(x); a[-5]=y; echo "same line"` prints nothing in both shells). Only the function call is different. The same happens with a readonly variable (`readonly r=1; f() { r=2; }; f; echo x`).

### Additional context

Found by an automated differential tester (brush vs bash 5.3); this report was drafted with AI assistance.
