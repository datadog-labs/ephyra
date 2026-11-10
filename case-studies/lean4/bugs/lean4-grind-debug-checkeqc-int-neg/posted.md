### Prerequisites

* [x] Check that your issue is not already filed: searched for `checkEqc`, `grind.debug` and `congruence root`. Related: #15871 (the same assertion on a `Bool` goal, fixed by #15874, which is in the failing builds).
* [x] Reduce the issue to a minimal, self-contained, reproducible test case. Avoid dependencies to Mathlib or Batteries.
* [x] Test your test case against the latest nightly release.

### Description

With `grind.debug` on, `grind` panics in the invariant checker on a goal it proves when the option is off:

```lean
set_option grind.debug true

example (x : Int8) (h : x = 1) : -x = -1 := by grind
```

```
lean repro.lean
repro.lean:3:47: error: <GoalM action default value>
PANIC at _private.Lean.Meta.Tactic.Grind.Inv.0.Lean.Meta.Grind.checkEqc Lean.Meta.Tactic.Grind.Inv:28:10: assertion violation: isSameExpr e ( __do_lift._@.Lean.Meta.Tactic.Grind.Inv.684779629._hygCtx._hyg.171.0 )
```

`Int16`, `Int32` and `Int64` panic the same way. `Int` does not. Without `set_option grind.debug true`, the file checks.

**Expected behavior:** the file checks, as it does without `grind.debug`.

**Actual behavior:** the assertion `isSameExpr e (← getCongrRoot curr)` in `checkEqc` (`Grind/Inv.lean:28`) fails, and `grind` reports `<GoalM action default value>`.

### Versions

```
4.37.0-nightly-2026-11-03   panics
4.37.0-nightly-2026-10-30   panics
master b21ac7949a            panics
4.36.0-rc2                   works
4.35.0                       works
```

These are on macOS arm64 (official toolchains via elan). The failing builds contain #15934, which classifies negative `Int8`..`Int64` numerals such as `-1` as literals; 4.36.0-rc2 does not.

### Impact

Add :+1: to issues you consider important. If others are impacted by this issue, please ask
them to add :+1: to it.

Found by an automated differential tester ([ephyra](https://github.com/datadog-labs/ephyra)); this report was drafted with AI assistance.
