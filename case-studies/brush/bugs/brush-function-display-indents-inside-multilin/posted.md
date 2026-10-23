# declare -f indents lines inside multi-line strings and here-documents, changing the function

Posted as https://github.com/reubeno/brush/issues/1580 on 2026-10-23. The text below is the issue as posted.

---

### brush version

brush 0.4.0 (git:b562e058), built from main at b562e0586b366d0558b27019276e591856253947 (2026-10-22).

### bash version (for comparison)

GNU bash 5.3

### Platform / OS

macOS

### Steps to reproduce

```bash
f() { echo "a
b"; }
eval "$(declare -f f)"; f
```

### Actual behavior (brush)

```
$ declare -f f
# brush:
f ()
{
    echo "a
    b"
}
$ eval "$(declare -f f)"; f
# brush:
a
    b
```

The second line of the string gets four spaces. After the round trip, the function prints something different.

Here-documents break the same way. The body and the closing delimiter get indented, so brush cannot read its own output back:

```bash
g() { cat <<EOF
hi
EOF
}
eval "$(declare -f g)"
# brush: error: unterminated here document; tag(s) `EOF` declared at ...
```

### Expected behavior

```
$ declare -f f
# bash:
f ()
{
    echo "a
b"
}
$ eval "$(declare -f f)"; f
# bash:
a
b
```

For `g`, bash prints `hi` and `EOF` with no indentation, and `eval "$(declare -f g)"; g` prints `hi`.

With a quoted delimiter (`cat <<'EOF'`), brush also prints the closing line with its quotes, as `'EOF'` (indented), so the delimiter never matches. bash prints `EOF`.

The bash manual says `set` shows functions "in a format that may be reused as input". For `export -pf`, it says "The '-p' option displays output in a form that may be reused as input". Reading the output back should not change what the function does.

### Additional context

Found by an automated differential tester (brush vs bash 5.3); this report was drafted with AI assistance.
