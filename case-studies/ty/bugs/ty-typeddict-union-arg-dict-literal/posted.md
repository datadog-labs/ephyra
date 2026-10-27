# Dict literal is rejected for a `TypedDict | dict[str, list[float]]` parameter: inner list inferred without context

Posted as https://github.com/astral-sh/ty/issues/4829 on 2026-10-26. The text below is the issue as posted.

---

### Summary

When a dict literal is passed where the expected type is a union of a TypedDict and a `dict` (or `Mapping`) with invariant container values, ty infers the inner `[1]` as `list[int]` instead of `list[float]`, and reports `invalid-argument-type`. Without the TypedDict in the union, the same call is accepted. The same wrong type is inferred for an annotated assignment, but there the error is only shown when the union has two TypedDicts. It seems the dict literal's items are first inferred without context while the TypedDict alternatives are tried, and that result is reused for the `dict` alternative instead of inferring again with `list[float]` as context. Scalar and tuple values are fine. This also happens for return values and attribute assignments.

## Minimal reproduction

```python
from typing import TypedDict


class TD(TypedDict):
    a: int


class TD2(TypedDict):
    b: str


def f(p: TD | dict[str, list[float]]) -> None: ...
def g(p: dict[str, list[float]]) -> None: ...


f({"x": [1]})                                   # error
g({"x": [1]})                                   # ok: no TypedDict in the union
x: TD | dict[str, list[float]] = {"x": [1]}     # no error
y: TD | TD2 | dict[str, list[float]] = {"x": [1]}   # error
```

```
ty check --python-version 3.13 --output-format concise repro.py
# ty 0.0.92:
repro.py:16:3: error[invalid-argument-type] Argument to function `f` is incorrect: Expected `TD | dict[str, list[float]]`, found `dict[str, list[int]]`
repro.py:19:40: error[invalid-assignment] Object of type `dict[str, list[int]]` is not assignable to `TD | TD2 | dict[str, list[float]]`

pyright --pythonversion 3.13 repro.py
# pyright 1.1.414: 0 errors, 0 warnings, 0 informations

mypy repro.py
# mypy 2.4.0: Success: no issues found in 1 source file
```

Possibly related: #3935 (same TypedDict-union dict-literal code path, different symptom).

## Playground

https://play.ty.dev/0c784ffa-c931-43f3-8f8d-7f867a9c3645

### Version

ty 0.0.92 (3933e6918 2026-10-26); also ruff main at e139fbd
