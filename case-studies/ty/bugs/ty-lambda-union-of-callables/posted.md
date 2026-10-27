# Lambda gets no type context when the expected type is a union of two or more `Callable`s

Posted as https://github.com/astral-sh/ty/issues/4835 on 2026-10-27. The text below is the issue as posted.

---

### Summary

When a lambda's expected type is a union of two or more `Callable` types, ty infers the lambda without any context: the parameters are `Unknown` and the body is inferred on its own. So `lambda s: [1]` becomes `(s) -> list[int]` and is rejected against `Callable[[str], list[float]] | Callable[[str], bool]`, although it fits the first member. With a single `Callable` (or `Callable | None`) the annotation is used as context, as the mdtest in `bidirectional.md` describes, and the same lambda is accepted. This affects assignments and call arguments alike.

## Minimal reproduction

```python
from collections.abc import Callable


x: Callable[[str], list[float]] | Callable[[str], bool] = lambda s: [1]   # error
y: Callable[[str], list[float]] = lambda s: [1]                           # ok


def f(p: Callable[[str], list[float]] | Callable[[str], bool]) -> None: ...
f(lambda s: [1])                                                          # error

z: Callable[[int], int] | Callable[[str], bool] = lambda a: reveal_type(a)
```

```
ty check --python-version 3.13 --output-format concise repro.py
# ty 0.0.92:
repro.py:4:59: error[invalid-assignment] Object of type `(s) -> list[int]` is not assignable to `((str, /) -> list[float]) | ((str, /) -> bool)`
repro.py:9:3: error[invalid-argument-type] Argument to function `f` is incorrect: Expected `((str, /) -> list[float]) | ((str, /) -> bool)`, found `(s) -> list[int]`
repro.py:11:73: info[revealed-type] Revealed type: `Unknown`

pyright --pythonversion 3.13 repro.py
# pyright 1.1.414: 0 errors (Type of "a" is "int")

mypy repro.py
# mypy 2.4.0: errors on lines 4 and 9 (it infers the lambda as Callable[[Any], list[int]]); Revealed type is "Any"
```

## Playground

https://play.ty.dev/ade2cbcf-af6a-41b3-b3fa-fd0a5e877a69


### Version

ty 0.0.92 (3933e6918 2026-10-26)
