# `TypeAliasType("A", value=int)` is rejected: "Wrong number of arguments … expected 2, found 1"

Posted as https://github.com/astral-sh/ty/issues/4854 on 2026-10-28. The text below is the issue as posted.

---

### Summary

`TypeAliasType` with `value` passed as a keyword argument is reported as `invalid-type-alias-type` ("expected 2, found 1"), and the alias is then treated as `Unknown`, so later uses aren't checked against it. Failing since 0.0.19.

## Minimal reproduction

```python
from typing import TypeAliasType

A = TypeAliasType("A", value=int)
B = TypeAliasType(name="B", value=int)
C = TypeAliasType("C", int)
x: A = 1
```

```
ty check --python-version 3.13 --output-format concise repro.py
# ty 0.0.93 (and main 691b60a):
repro.py:3:5: error[invalid-type-alias-type] Wrong number of arguments in `TypeAliasType` creation: expected 2, found 1
repro.py:4:5: error[invalid-type-alias-type] Wrong number of arguments in `TypeAliasType` creation: expected 2, found 0
# ty 0.0.18: All checks passed!

pyright --pythonversion 3.13 repro.py
# pyright 1.1.414: 0 errors, 0 warnings, 0 informations

mypy repro.py
# mypy 2.4.0: repro.py:4: error: TypeAliasType() expects a string literal as first argument  [misc]
#   (line 3, `value=` as a keyword, is accepted)

python3.13 repro.py
# runs without error
```

## Playground

https://play.ty.dev/3d187ccf-43a3-4159-9c80-f4b4d11e8dc9

### Version

ty 0.0.93 (3db5e4df7 2026-10-28)
