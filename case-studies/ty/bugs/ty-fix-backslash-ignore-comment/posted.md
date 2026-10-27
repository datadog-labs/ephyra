# `--fix` for `unused-ignore-comment` after a backslash continuation introduces a syntax error, so no fixes in the file are applied

Posted as https://github.com/astral-sh/ty/issues/4840 on 2026-10-27. The text below is the issue as posted.

---

### Summary

When an unused `# ty: ignore[...]` comment sits on a continuation line after a backslash, the fix for `unused-ignore-comment` removes that whole line. That joins the backslash-continued statement with the next statement, which is a syntax error. ty detects it and reverts with `fatal[internal-error] Applying fixes introduced a syntax error. Reverting changes.`, so none of the fixes in that file are applied. 

## Minimal reproduction

```python
x = 1  \
    # ty: ignore[invalid-assignment]
y = 2
```

```
ty check --output-format concise --no-progress repro.py
# ty 0.0.92 and main (691b60a):
repro.py:2:5: warning[unused-ignore-comment] Unused `ty: ignore` directive

ty check --output-format concise --no-progress --fix repro.py
# ty 0.0.92 and main (691b60a):
repro.py: fatal[internal-error] Applying fixes introduced a syntax error. Reverting changes.
repro.py:2:5: warning[unused-ignore-comment] Unused `ty: ignore` directive
Found 2 diagnostics (0 fixed, 2 remaining).

python3.13 repro.py
# runs without error
```

Found with 


### Version

ty 0.0.92 (3933e6918 2026-10-26)
