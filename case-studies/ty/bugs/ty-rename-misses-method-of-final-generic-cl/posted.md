# Rename skips calls of a method of a `@final` generic class through a specialized instance

Posted as https://github.com/astral-sh/ty/issues/4860 on 2026-10-28. The text below is the issue as posted.

---

### Summary

Renaming a method of a `@final` generic class does not edit calls made through a specialized instance, e.g. `c.value()` with `c: C[int]`. Find references does return that call, and go to definition from the call works, but rename misses it; renaming from the call site itself returns no edits at all. After the rename the file has an `unresolved-attribute` error and fails at runtime.

## Minimal reproduction

```python
from typing import final


@final
class C[T]:
    def value(self) -> int:
        return 1


def f(c: C[int]) -> int:
    return c.value()


print(f(C()))
```

Rename `value` to `renamed` with `ty server` (positions 1-based):

```
# ty 0.0.93 (and main 691b60a), rename at the declaration (line 6, column 9):
textDocument/references: (6, 9), (11, 14)
textDocument/rename edits: (6, 9)          # (11, 14) is missing

# rename at the call site (line 11, column 14):
textDocument/definition: (6, 9)
textDocument/references: (6, 9), (11, 14)
textDocument/rename edits: (none)

# after the rename at the declaration:
ty check --python-version 3.13 --output-format concise renamed.py
renamed.py:11:12: error[unresolved-attribute] Object of type `C[int]` has no attribute `value`

python3.13 renamed.py
# AttributeError: 'C' object has no attribute 'value'
```

### Version

ty 0.0.93 (3db5e4df7 2026-10-28)
