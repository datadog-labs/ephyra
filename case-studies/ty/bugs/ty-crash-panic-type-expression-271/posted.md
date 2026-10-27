# [panic] Chained assignment with a missing value (debug assertions)

Posted as https://github.com/astral-sh/ty/issues/4837 on 2026-10-27. The text below is the issue as posted.

---

### Summary

A chained assignment whose value is missing (`X = Y =`, a syntax error), with one of its targets later used in an annotation, triggers a debug assertion in `type_expression.rs`. Release builds only report the syntax error. Builds with debug assertions (as in ty's own tests) panic, and the file is not checked. A plain `X =` or a complete `X = Y = int` is fine; the target can be used in any annotation (`x: X`, `def f(x: Y)`).

## Minimal reproduction

```python
X = Y =
x: X
```

```
CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true cargo build --release -p ty
ty check --python-version 3.13 --output-format concise repro.py
# ty main at e139fbd, debug assertions on:
fatal[panic] Panicked at crates/ty_python_semantic/src/types/infer/builder/type_expression.rs:271:22 when checking `repro.py`: `Calling `self.infer_expression` on a standalone-expression is not allowed because it can lead to double-inference. Use `self.infer_standalone_expression` instead.`
WARN A fatal error occurred while checking some files. Not all project files were analyzed. See the diagnostics list above for details.

# ty 0.0.92 (release):
repro.py:1:8: error[invalid-syntax] Expected an expression
```

### Version

ty main at e139fbd (debug assertions)
