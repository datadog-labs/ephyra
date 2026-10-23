# read drops control characters from a pipe, file or here-string, and treats ^C/^D in the data as keystrokes

Posted as https://github.com/reubeno/brush/issues/1579 on 2026-10-23. The text below is the issue as posted.

---

### brush version

brush 0.4.0 (git:b562e058), built from main at b562e0586b366d0558b27019276e591856253947 (2026-10-22).

### bash version (for comparison)

GNU bash 5.3.20(1)-release

### Platform / OS

macOS

### Steps to reproduce

```sh
read -r v <<< $'a\x01b'; printf '%s' "$v" | od -An -c
printf 'x\003y\n' | { read -r v; echo "status=$? v=$v"; }
printf 'x\004y\n' | { read -r v; echo "status=$? v=$v"; }
```

Run as `brush --norc --noprofile poc.sh` and `bash --norc --noprofile poc.sh`, with `LC_ALL=C`.

### Actual behavior (brush)

```sh
read -r v <<< $'a\x01b'; printf '%s' "$v" | od -An -c
# brush:    a   b
printf 'x\003y\n' | { read -r v; echo "status=$? v=$v"; }
# brush: status=1 v=
printf 'x\004y\n' | { read -r v; echo "status=$? v=$v"; }
# brush: status=0 v=x
```

The `\x01` byte is silently removed. A `\x03` byte makes read fail with status 1 and an empty
variable, as if Ctrl-C was pressed. A `\x04` byte ends the line, as if Ctrl-D was pressed.
Other control bytes such as ESC (`\x1b`) and DEL (`\x7f`) are removed too. For example,
`read -r line < file` on a line with ANSI color codes loses the ESC bytes.

### Expected behavior

```sh
read -r v <<< $'a\x01b'; printf '%s' "$v" | od -An -c
# bash:    a 001   b
printf 'x\003y\n' | { read -r v; echo "status=$? v=$v"; }
# bash: status=0 v=x<0x03>y
printf 'x\004y\n' | { read -r v; echo "status=$? v=$v"; }
# bash: status=0 v=x<0x04>y
```

The bash manual says for `read`: "One line is read from the standard input, or from the file descriptor fd
supplied as an argument to the -u option, split into words as described above in Word Splitting, and the first
word is assigned to the first name". Only backslash (without `-r`), IFS characters and the delimiter are special.
When the input is not a terminal, other bytes should reach the variable unchanged.

### Additional context

Found by an automated differential tester (brush vs bash 5.3); this report was drafted with AI assistance.
