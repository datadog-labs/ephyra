# Invalid edit offer on variable-type inlay hint

Posted as https://github.com/astral-sh/ty/issues/4859 on 2026-10-28. The text below is the issue as posted.

---

### Summary

For an assignment to an attribute of an object other than `self`/`cls` (here a function parameter), the language server shows a `: int` variable-type hint with a text edit. Accepting it inserts `c.a: int = len(s)`, which `ty check` then reports as `invalid-type-form` ("Type annotations are not allowed on this attribute expression").

## Minimal reproduction

```python
def f(c, s: str):
    c.a = len(s)
```

`textDocument/inlayHint` from `ty server`, then applying the returned `textEdits`:

```
# ty 0.0.93 (and main 691b60a):
hint ': int' at line 2, column 8; textEdits [(': int', 8)]

# after accepting:
def f(c, s: str):
    c.a: int = len(s)

ty check --python-version 3.13 --output-format concise repro.py   # the edited file
repro.py:2:10: error[invalid-type-form] Type annotations are not allowed on this attribute expression
```

### Version

ty 0.0.93 (3db5e4df7 2026-10-28)
