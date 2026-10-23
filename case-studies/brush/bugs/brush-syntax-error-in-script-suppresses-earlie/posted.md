# A syntax error later in a script file or -c string stops every earlier command from running

Posted as https://github.com/reubeno/brush/issues/1578 on 2026-10-23. The text below is the issue as posted.

---

### brush version

brush 0.4.0 (git:b562e058), built from main at b562e0586b366d0558b27019276e591856253947 (2026-10-22).

### bash version (for comparison)

GNU bash, version 5.3.20(1)-release

### Platform / OS

macOS

### Steps to reproduce

```bash
printf 'echo before\nfi\n' > t.sh
brush t.sh; echo "exit=$?"
brush -c $'echo before\nfi'; echo "exit=$?"
```

### Actual behavior (brush)

```
brush t.sh; echo "exit=$?"
# brush: error: t.sh: syntax error at line 2 col 3
# brush: exit=2
brush -c $'echo before\nfi'; echo "exit=$?"
# brush: error: -c: syntax error at end of input
# brush: exit=2
```

`echo before` never runs.

### Expected behavior

```
bash t.sh; echo "exit=$?"
# bash: before
# bash: t.sh: line 2: syntax error near unexpected token `fi'
# bash: t.sh: line 2: `fi'
# bash: exit=2
bash -c $'echo before\nfi'; echo "exit=$?"
# bash: before
# bash: bash: -c: line 2: syntax error near unexpected token `fi'
# bash: bash: -c: line 2: `fi'
# bash: exit=2
```

The bash manual (INVOCATION) says: "Bash reads and executes commands from this file, then exits." Commands before the bad line run first.

brush already does this for a script piped on stdin: `brush < t.sh` prints `before` and then the error. Its own compat test `brush-shell/tests/cases/compat/piped_script.yaml` ("Syntax error mid-stream") expects that. Script files and `-c` behave differently.

### Additional context

Found by an automated differential tester (brush vs bash 5.3); this report was drafted with AI assistance.
