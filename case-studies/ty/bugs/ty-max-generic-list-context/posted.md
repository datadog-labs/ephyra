# False positive when a generic call over `list[T]` is passed to a bounded TypeVar parameter

Posted as https://github.com/astral-sh/ty/issues/4828 on 2026-10-26. The text below is the issue as posted.

---

### Summary

`x: int = max(first([1]), 2)` is reported as `invalid-assignment`, with `def first[T](x: list[T]) -> T`. ty seems to use the outer function's TypeVar bound (`SupportsRichComparison` for `max`, `object` for a plain `S: object`) as type context for the inner call, instead of the declared `int`. The `list[T]` argument is widened to the bound, so `T` resolves to the bound and the result no longer fits `int`.

## Minimal reproduction

```python
from collections.abc import Sequence


def first[T](x: list[T]) -> T:
    return x[0]


def first_seq[T](x: Sequence[T]) -> T:
    return x[0]


def ident[S: object](a: S) -> S:
    return a


def ident_int[S: int](a: S) -> S:
    return a


a: int = max(first([1]), 2)        # error
b: int = ident(first([1]))         # error
c: int = ident(first_seq([1]))     # ok: Sequence[T] instead of list[T]
d: int = ident_int(first([1]))     # ok: bound int
e = ident(first([1]))              # no error, but:
reveal_type(e)
```

```
ty check --python-version 3.13 --output-format concise repro.py
# ty 0.0.92:
repro.py:20:10: error[invalid-assignment] Object of type `SupportsDunderLT[Any] | Literal[2]` is not assignable to `int`
repro.py:21:10: error[invalid-assignment] Object of type `object` is not assignable to `int`
repro.py:25:13: info[revealed-type] Revealed type: `object`

pyright --pythonversion 3.13 repro.py
# pyright 1.1.414: 0 errors, 0 warnings, 1 information (Type of "e" is "int")

mypy repro.py
# mypy 2.4.0: Success: no issues found in 1 source file (Revealed type is "int")
```

## Playground

https://play.ty.dev/3a6b9a2c-e276-4235-9216-941a5efaa73e

### Version

ty 0.0.92 (3933e6918 2026-10-26)
