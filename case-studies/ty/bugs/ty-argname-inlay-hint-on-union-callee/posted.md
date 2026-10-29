# Call-argument-name inlay hint on a union callee uses the first element's parameter name

Posted as https://github.com/astral-sh/ty/issues/4858 on 2026-10-28. The text below is the issue as posted.

---

### Summary

When the callee is a union of callables with different parameter names, the call-argument-name inlay hint shows the name from the first element only.

If the user accepts the hint's text edit for the example below (`(f if flag else g)(a=1)`), the result is a `TypeError` in runtime when `g` is the one called.

## Minimal reproduction

```python
def f(a): ...
def g(b): ...

def h(flag: bool):
    (f if flag else g)(1)

h(False)
```

`textDocument/inlayHint` from `ty server` for the whole file, then applying the returned `textEdits`:

```
# ty 0.0.93 (and main 691b60a):
hint 'a=' at line 5, column 24; textEdits [('a=', 24)]

# after accepting: (f if flag else g)(a=1)
ty check --python-version 3.13 --output-format concise edited.py
edited.py:5:5: error[missing-argument] No argument provided for required parameter `b` of function `g`
edited.py:5:24: error[unknown-argument] Argument `a` does not match any known parameter of function `g`

python3.13 edited.py
# TypeError: g() got an unexpected keyword argument 'a'
```


### Version

ty 0.0.93 (3db5e4df7 2026-10-28)
