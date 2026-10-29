# Accepting a call-argument-name inlay hint can produce a syntax error (`f((1))` → `f((x=1))`, `f(1, 2)` → `f(x=1, 2)`)

Posted as https://github.com/astral-sh/ty/issues/4855 on 2026-10-28 (one issue with ty-argname-inlay-hint-edit-before-later-pos). The text below is the issue as posted.

---

### Summary

The text edit attached to call-argument-name inlay hints (applied when the hint is accepted, e.g. double-clicked) produces invalid code in two cases:
1. For a parenthesized positional argument, the hint and its edit are placed inside the parentheses: `f((1))` becomes `f((x=1))`. Placing it before the opening parenthesis would give `f(x=(1))`.
2. When a later positional argument has no hint (here it binds to `*rest`), the edit is still offered on the earlier argument: `f(1, 2)` becomes `f(x=1, 2)`, a keyword argument followed by a positional one.

`ty check` is unaffected; this is `textDocument/inlayHint` in `ty server` with the default `ty.inlayHints.callArgumentNames`.

## Minimal reproduction

```python
def f(x): ...
f((1))
```

```python
def f(x, *rest): ...
f(1, 2)
```

Requesting `textDocument/inlayHint` for each file from `ty server` and applying the returned `textEdits`:

```
# ty 0.0.93 (and main 691b60a), first file:
hint 'x=' at line 2, column 4; textEdits [('x=', 4)]
after accepting: f((x=1))
SyntaxError: invalid syntax

# ty 0.0.93 (and main 691b60a), second file:
hint 'x=' at line 2, column 3; textEdits [('x=', 3)]
after accepting: f(x=1, 2)
SyntaxError: positional argument follows keyword argument
```

### Version

ty 0.0.93 (3db5e4df7 2026-10-28)
