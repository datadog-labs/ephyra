### Prerequisites

* [x] Check that your issue is not already filed: searched for `getOptions recording dependencies`, `getOptions called inside a computation`, `checkedOptions` and `warningAsError`.
* [x] Reduce the issue to a minimal, self-contained, reproducible test case. Avoid dependencies to Mathlib or Batteries.
* [x] Test your test case against the latest nightly release.

### Description

Elaborating this one-line example panics:

```lean
example : decide ((BitVec.twoPow 276 0).toFin = Fin.ofNat (2 ^ 276) 1) = true := by decide
```

```
warning: exponent 276 exceeds the threshold 256, exponentiation operation was not evaluated, use `set_option exponentiation.threshold <num>` to set a new threshold
PANIC at Lean.Core.instMonadOptionsCoreM.checkedOptions Lean.CoreM:427:6: `getOptions` called inside a computation recording its dependencies; use `Lean.getRecordedOption` for reads that can influence the result and `getOptionsUnrestricted` for all others
```

The exponent warning is logged during type class synthesis, while `synthInstance` records
the options its result depends on. `logAt` (`src/Lean/Log.lean`) decides whether to promote
the warning with `warningAsError.get (← getOptions)`. That plain `getOptions` read is what
`checkedOptions` rejects inside a dependency-recording computation. Message rendering cannot
influence the cached result, so this read is one the panic message itself says should use
`getOptionsUnrestricted`.

**Expected behavior:** the warning is reported, with no panic.

**Actual behavior:** the warning, then the `PANIC`.

### Versions

```
4.37.0-nightly-2026-10-30   panics
master b21ac7949a            panics
4.35.0                       no panic
```

These are on macOS arm64 (official toolchains via elan).


### Impact

Add :+1: to issues you consider important. If others are impacted by this issue, please ask
them to add :+1: to it.

Found by an automated differential tester ([ephyra](https://github.com/datadog-labs/ephyra)); this report was drafted with AI assistance.
