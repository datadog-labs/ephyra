# Literal string compared with LiteralString narrows to Never in the != branch

Posted as https://github.com/astral-sh/ty/issues/4826 on 2026-10-26. The text below is the issue as posted.

---

### Summary

When a variable holding a string literal is compared with a LiteralString value, ty narrows the variable to Never in the != branch, and in the else branch of ==. That branch does run at runtime, since "1" != "42", but it is not checked: v + 1 raises a TypeError at runtime and ty reports no error. As far as I can tell it started in 0.0.50, 0.0.49 reports the error correctly.

## Minimal reproduction

```python
def f() -> None:
    v = "1"
    if v != "42".upper():  # True at runtime
        v + 1              # TypeError at runtime


def g() -> None:
    v = "1"
    if v != "42".upper():
        reveal_type(v)


f()
```

```
ty check --python-version 3.13 --output-format concise repro.py
# ty 0.0.92 (also with strict-equality-semantics = true):
repro.py:10:21: info[revealed-type] Revealed type: `Never`
# no error on line 4

# ty 0.0.49:
repro.py:4:9: error[unsupported-operator] Operator `+` is not supported between objects of type `Literal["1"]` and `Literal[1]`
repro.py:10:21: info[revealed-type] Revealed type: `Literal["1"]`

pyright --pythonversion 3.13 repro.py
# pyright 1.1.414:
repro.py:4:9 - error: Operator "+" not supported for types "Literal['1']" and "Literal[1]" (reportOperatorIssue)

mypy repro.py
# mypy 2.4.0:
repro.py:4: error: Unsupported operand types for + ("str" and "int")  [operator]
repro.py:10: note: Revealed type is "str"

python3.13 repro.py
# TypeError: can only concatenate str (not "int") to str
```

## Playground

https://play.ty.dev/dbef9e6a-1942-4ae8-819d-02c33ebd6167

### Version

ty 0.0.92 (3933e6918 2026-10-26)
