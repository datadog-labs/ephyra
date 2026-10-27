# Generic alias parameter `A[T]` (`type A[T] = list[T]`) gets no context from sibling arguments in a generic call

Posted as https://github.com/astral-sh/ty/issues/4830 on 2026-10-26. The text below is the issue as posted.

---

### Summary

In a generic call, an argument for a parameter annotated with a generic type alias, `x: A[T]` with `type A[T] = list[T]`, is not inferred again using the solution from the other arguments, as it is for `x: list[T]`. Here the comprehension's condition is statically false, so on its own the comprehension is `list[Never]`. With `list[T]` it is re-inferred as `list[C]` from `y: T`. With the alias it stays `list[Never]` and is rejected.

## Minimal reproduction

```python
class C: ...


type A[T] = list[T]


def f[T](x: A[T], y: T) -> T: return y
def g[T](x: list[T], y: T) -> T: return y


f([C() for s in ["a"] if s == "b"], C())  # error
g([C() for s in ["a"] if s == "b"], C())  # ok
```

```
ty check --python-version 3.13 --output-format concise repro.py
# ty 0.0.92:
repro.py:11:3: error[invalid-argument-type] Argument to function `f` is incorrect: Expected `A[C]`, found `list[Never]`

pyright --pythonversion 3.13 repro.py
# pyright 1.1.414: 0 errors, 0 warnings, 0 informations

mypy repro.py
# mypy 2.4.0: Success: no issues found in 1 source file
```

## Playground

https://play.ty.dev/00fbcb5d-4dc6-4aa7-94ee-ac9830ae5d58

### Version

ty 0.0.92 (3933e6918 2026-10-26)
