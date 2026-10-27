# [panic] `--update-baseline` aborts: "user-provided comparison function does not correctly implement a total order"

Posted as https://github.com/astral-sh/ty/issues/4839 on 2026-10-27. The text below is the issue as posted.

---

### Summary

`ty check --update-baseline --baseline base.json` aborts the whole process on a small project of two files with syntax errors and an unused `ty: ignore`. The panic is Rust's sort detecting an inconsistent comparator, in `CollectReporter::into_sorted` (`ty_project/src/lib.rs`), which sorts diagnostics with `RenderingSortKey::cmp` (`ruff_db/src/diagnostic/mod.rs`).

## Minimal reproduction

Two files in an otherwise empty directory (leading spaces are part of the input):

`r_1.py`
```python
        _probe(19, x23)
                    _probe(21,
                    v27: tuple[()] = ()  # ty:ignore[possibly-unresolved-reference]
    return p1
```

`r_5.py`
```python
    k0_0: typing.List[Annotated[bool, "m17"]]
    k0_1: ReadOnly[tuple[()]]
class TD1(TypedDict):
    k1_0: TD0  # ty: ignore[unresolved-attribute]  # trailing
```

```
ty check --output-format concise --no-progress --project . --update-baseline --baseline base.json
# ty main at 691b60a (release and debug builds), every run:
thread '<unnamed>' panicked at library/core/src/slice/sort/shared/smallsort.rs:854:5:
user-provided comparison function does not correctly implement a total order
Rayon: detected unexpected panic; aborting
# exit code 134; no base.json written

ty check --output-format concise --no-progress --project .
# same files, no baseline: no panic, diagnostics as expected
```

### Version

ty main at 691b60a (2026-10-27)
