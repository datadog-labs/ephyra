### Prerequisites

* [x] Check that your issue is not already filed: searched for `linarith term has not been internalized`, `term has not been internalized`, `grind linarith internal error` and `grind Int8 negative literal`. Related: #16229 (same root change, `grind.debug` checker; fixed by #16263, which does not cover this).
* [x] Reduce the issue to a minimal, self-contained, reproducible test case. Avoid dependencies to Mathlib or Batteries.
* [x] Test your test case against the latest nightly release.

### Description

`grind_linarith` raises an internal error on `Int8`..`Int64` goals with a negative literal:

```lean
example (x : Int8) (h : -1 ≤ x) : -2 ≤ x := by grind_linarith
```

```
lean NegLit.lean
error: `grind` internal error, linarith term has not been internalized
  1
```

`Int16`, `Int32` and `Int64` behave the same way. `Int` works, and plain `grind` proves the `Int8` goal.

This looks like the same root change as #16229: since #15934, negative `IntN` literals are classified as literals and are not internalized the usual way. #16263 fixed the `grind.debug` checker for #16229. It moves `getLitKind` from `Internalize.lean` to `Types.lean` without changing it, so it does not affect this error.

**Expected behavior:** the goal is proved, as on 4.36.0-rc2.

**Actual behavior:** an internal error.

### Versions

```
4.37.0-nightly-2026-11-04   internal error
4.36.0-rc2                   works
4.35.0                       works
```

These are on macOS arm64 (official toolchains via elan).

### Impact

Add :+1: to issues you consider important. If others are impacted by this issue, please ask
them to add :+1: to it.

Found by an automated differential tester ([ephyra](https://github.com/datadog-labs/ephyra)); this report was drafted with AI assistance.
