# Conditional of list literals passed to `list[T]` is rejected when a sibling argument also binds `T`, depending on branch order

Posted as https://github.com/astral-sh/ty/issues/4836 on 2026-10-27. The text below is the issue as posted.

---

### Summary

In a generic call where another argument already constrains `T`, an argument that is a conditional expression of collection literals is inferred branch by branch against the already-specialized parameter type.

## Minimal reproduction

```python
class A: ...
class B(A): ...


def f[T](items: list[T], item: T) -> None: ...
def g[T](items: set[T], item: T) -> None: ...


def check(flag: bool) -> None:
    f([A()] if flag else [B()], B())   # error
    f([B()] if flag else [A()], B())   # ok: branches swapped
    f([A(), B()], B())                 # ok: one list literal
    g({A()} if flag else {B()}, B())   # error: sets too


check(True)
```

```
ty check --python-version 3.13 --output-format concise repro.py
# ty 0.0.92:
repro.py:10:7: error[invalid-argument-type] Argument to function `f` is incorrect: Expected `list[B]`, found `list[A] | list[B]`
repro.py:13:7: error[invalid-argument-type] Argument to function `g` is incorrect: Expected `set[B]`, found `set[A] | set[B]`

pyright --pythonversion 3.13 repro.py
# pyright 1.1.414: 0 errors, 0 warnings, 0 informations

mypy repro.py
# mypy 2.4.0: "Cannot infer value of type parameter" on lines 10, 11 and 13 (both orders)

python3.13 repro.py
# runs without error
```

## Playground

https://play.ty.dev/5f5d03dc-d095-4dfd-b4cc-fe12dc6d873b

### Version

ty 0.0.92 (3933e6918 2026-10-26)
