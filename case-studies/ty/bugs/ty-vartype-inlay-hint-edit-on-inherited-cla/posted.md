# Accepting the variable-type inlay hint on a subclass override of a `ClassVar` gives `invalid-attribute-override`

Posted as https://github.com/astral-sh/ty/issues/4866 on 2026-10-29. The text below is the issue as posted.

---

### Summary

In a class body, an unannotated assignment that overrides a base class's `ClassVar` gets a `: int` variable-type inlay hint with a text edit. Accepting the hint inserts a plain annotation (`x: int = ...`), which turns the class variable into an instance variable, and `ty check` then reports `invalid-attribute-override`. 

## Minimal reproduction

```python
from typing import ClassVar

class Base:
    x: ClassVar[int] = 0

class Child(Base):
    x = abs(-2)
```

`textDocument/inlayHint` from `ty server`, then applying the returned `textEdits`:

```
# ty 0.0.94 (and main 691b60a): the `: int` hint on line 7 carries an edit; after accepting:
class Child(Base):
    x: int = abs(-2)

ty check --python-version 3.13 --output-format concise repro.py   # the edited file
repro.py:7:5: error[invalid-attribute-override] Invalid override of attribute `x`: instance variable cannot override class variable `Base.x`
```

### Version

ty 0.0.94 (f8ea270e0 2026-10-28)
