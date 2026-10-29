# `unknown-argument` for extra keyword arguments to a pydantic-settings `BaseSettings` model with `extra="allow"`

Posted as https://github.com/astral-sh/ty/issues/4865 on 2026-10-29. The text below is the issue as posted.

---

### Summary

A `pydantic_settings.BaseSettings` subclass with `model_config = SettingsConfigDict(extra="allow")` accepts extra keyword arguments at runtime and stores them on the model, but ty reports `unknown-argument` for them.
## Minimal reproduction

```python
from pydantic_settings import BaseSettings, SettingsConfigDict


class Settings(BaseSettings):
    model_config = SettingsConfigDict(extra="allow")

    name: str


s = Settings(name="app", debug=True)
print(s)
```

Environment: Python 3.13 venv with pydantic 2.14.1 and pydantic-settings 2.16.2.

```
ty check --python <venv> --python-version 3.13 --output-format concise repro.py
# ty 0.0.94 (and main 691b60a):
repro.py:10:26: error[unknown-argument] Argument `debug` does not match any known parameter

python3.13 repro.py
# name='app' debug=True

mypy --python-version 3.13 repro.py      # with plugins = pydantic.mypy
# mypy 2.4.1: Success: no issues found in 1 source file
```

### Version

ty 0.0.94 (f8ea270e0 2026-10-28)
