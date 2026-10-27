# Narrowing lost in a comprehension inside a nested function

Posted as https://github.com/astral-sh/ty/issues/4825 on 2026-10-26. The text below is the issue as posted.

---

### Summary

ty misses a narrowing opportunity inside a comprehension nested in an inner function, producing a false positive. The same narrowing is kept directly in the inner function, and in a comprehension directly in the narrowing scope.

## Minimal reproduction

```python
def f(p: int | None) -> None:
    if p is not None:
        xs = [p < 1 for _ in range(2)]        # ok: comprehension directly in f
        def inner() -> list[bool]:
            ok = p < 1                         # ok: directly in inner
            return [p < 1 for _ in range(2)]   # error


def outer() -> None:
    x = None
    x = 1
    def g() -> list[int]:
        return [x + 1 for _ in range(2)]
```

The second function shows the same lookup also misses which bindings are visible, not only narrowing: `x` is `Literal[1]` directly in `g`, but `None | Literal[1]` in the comprehension.

```
ty check --python-version 3.13 --output-format concise repro.py
# ty 0.0.92:
repro.py:6:21: error[unsupported-operator] Operator `<` is not supported between objects of type `int | None` and `Literal[1]`
repro.py:13:17: error[unsupported-operator] Operator `+` is not supported between objects of type `None | Literal[1]` and `Literal[1]`
Found 2 diagnostics

pyright --pythonversion 3.13 repro.py
# pyright 1.1.414: 0 errors, 0 warnings, 0 informations

mypy repro.py
# mypy 2.4.0: Success: no issues found in 1 source file
```

## Playground

https://play.ty.dev/d7133400-114b-493f-a974-3481db62a4e2


### Version

ty 0.0.92 (3933e6918 2026-10-26); also ruff main at e139fbd
