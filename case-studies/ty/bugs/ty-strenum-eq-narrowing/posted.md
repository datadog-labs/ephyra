# `x == S.A` narrows a `str` to the StrEnum, although `"a" == S.A`

Posted as https://github.com/astral-sh/ty/issues/4827 on 2026-10-26. The text below is the issue as posted.

---

### Summary

When a `str` is compared with a `StrEnum` member, ty narrows it to the enum in the `==` branch. The same happens with `in` and with a `case S.A:` pattern, and with an `int` against an `IntEnum` member. But a `StrEnum` member compares equal to its plain value (`"a" == S.A` is `True`), so a plain string reaches that branch: `x.name` passes ty and raises `AttributeError` at runtime. With `strict-equality-semantics = true` ty is correct. It started in 0.0.63; 0.0.62 reports the error.

## Minimal reproduction

```python
from enum import IntEnum, StrEnum


class S(StrEnum):
    A = "a"


class I(IntEnum):
    ONE = 1


def f(x: str) -> str:
    if x == S.A:
        return x.name  # AttributeError at runtime: x is the str "a"
    return x


def g(x: str, y: int) -> None:
    if x == S.A:
        reveal_type(x)
    match x:
        case S.A:
            reveal_type(x)
    if y == I.ONE:
        reveal_type(y)


f("a")
```

```
ty check --python-version 3.13 --output-format concise repro.py
# ty 0.0.92 (no error on line 14):
repro.py:20:21: info[revealed-type] Revealed type: `S`
repro.py:23:25: info[revealed-type] Revealed type: `S`
repro.py:25:21: info[revealed-type] Revealed type: `I`

# ty 0.0.62, and 0.0.92 with strict-equality-semantics = true:
repro.py:14:16: error[unresolved-attribute] Object of type `str` has no attribute `name`
repro.py:20:21: info[revealed-type] Revealed type: `str`
repro.py:23:25: info[revealed-type] Revealed type: `str`
repro.py:25:21: info[revealed-type] Revealed type: `int`

pyright --pythonversion 3.13 repro.py
# pyright 1.1.414:
repro.py:14:18 - error: Cannot access attribute "name" for class "str"
repro.py:20:21 - information: Type of "x" is "str"
repro.py:23:25 - information: Type of "x" is "Literal[S.A]"
repro.py:25:21 - information: Type of "y" is "int"

mypy repro.py
# mypy 2.4.0:
repro.py:14: error: "str" has no attribute "name"  [attr-defined]

python3.13 repro.py
# AttributeError: 'str' object has no attribute 'name'
```

## Playground

https://play.ty.dev/8580be8e-45a3-4c61-9a5f-dcbfba711118

### Version

ty 0.0.92 (3933e6918 2026-10-26)
