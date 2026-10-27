# Mapping-pattern capture is `object` when the subject was narrowed to an intersection (e.g. after `if d:`)

Posted as https://github.com/astral-sh/ty/issues/4831 on 2026-10-26. The text below is the issue as posted.

---

### Summary

After `if d:` on a `dict[str, int]`, ty narrows `d` to `dict[str, int] & ~AlwaysFalsy`, and a capture in a mapping pattern on it (`case {"a": c}`) is inferred as `object` instead of `int`. Started in 0.0.55; earlier releases inferred `Unknown` here.

## Minimal reproduction

```python
def f(d: dict[str, int]) -> None:
    if d:
        match d:
            case {"a": c}:
                print(c.bit_length())   # error


def g(d: dict[str, int]) -> None:
    match d:
        case {"a": c}:
            print(c.bit_length())       # ok


f({"a": 1})
g({"a": 1})
```

```
ty check --python-version 3.13 --output-format concise repro.py
# ty 0.0.92:
repro.py:5:23: error[unresolved-attribute] Object of type `object` has no attribute `bit_length`

pyright --pythonversion 3.13 repro.py
# pyright 1.1.414: 0 errors, 0 warnings, 0 informations

mypy repro.py
# mypy 2.4.0: Success: no issues found in 1 source file

python3.13 repro.py
# 1
# 1
```

## Playground

https://play.ty.dev/60823bc7-8676-4ad0-9757-abf3a6b36308

### Version

ty 0.0.92 (3933e6918 2026-10-26)
