# Rename / find references of an attribute skip accesses in platform-dependent unreachable code

Posted as https://github.com/astral-sh/ty/issues/4856 on 2026-10-28. The text below is the issue as posted.

---

### Summary

Renaming an attribute (or finding its references) in the language server leaves out `obj.attr` accesses in code that ty considers unreachable for the current platform, e.g. under `if sys.platform == "win32":` on macOS or Linux. After the rename, that branch still uses the old name, so the file is broken on the platform where the branch runs.

## Minimal reproduction

```python
import sys


class C:
    x: int = 1


def f(c: C) -> int:
    if sys.platform == "win32":
        return c.x
    return c.x
```

Rename `x` (line 5) to `y` with `ty server` on macOS:

```
# ty 0.0.93 (and main 691b60a):
textDocument/references: (5, 5), (11, 14)    # (10, 16) is missing
textDocument/rename edits: (5, 5), (11, 14)

# the renamed file:
#     if sys.platform == "win32":
#         return c.x
#     return c.y

ty check --python-version 3.13 --python-platform win32 --output-format concise renamed.py
renamed.py:10:16: error[unresolved-attribute] Object of type `C` has no attribute `x`
```

### Version

ty 0.0.93 (3db5e4df7 2026-10-28)
