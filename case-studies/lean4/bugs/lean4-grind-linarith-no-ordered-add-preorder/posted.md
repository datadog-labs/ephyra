### Prerequisites

* [x] Check that your issue is not already filed: searched for `structure is not a preorder`, `grind linarith internal error`, `grind_linarith Fin`, `grind linarith BitVec` and `OrderedAdd`. Related: #15675 (a similar guard for a missing `LawfulOrderLT`).
* [x] Reduce the issue to a minimal, self-contained, reproducible test case. Avoid dependencies to Mathlib or Batteries.
* [x] Test your test case against the latest nightly release.

### Description

`grind_linarith` raises an internal error on a strict inequality over `Fin n` or `BitVec w`, types with an order and an `IntModule` instance but no `Grind.OrderedAdd` instance:

```lean
example (x : Fin 3) (h : x < x) : False := by grind_linarith
example (x : BitVec 8) (h : x < x) : False := by grind_linarith
```

```
lean Preorder.lean
error: `grind linarith` internal error, structure is not a preorder
```

Plain `grind` proves `example (x : Fin 3) (h : x < x) : False`. `grind -ring -lia` gives the same internal error.

**Expected behavior:** `grind_linarith` proves the goal, or fails with its normal failure message.

**Actual behavior:** an internal error.

### Versions

```
4.37.0-nightly-2026-11-04   internal error
4.36.0-rc2                   internal error
4.35.0                       internal error
```

These are on macOS arm64 (official toolchains via elan). Master after the nightly has no commits under `src/Lean/Meta/Tactic/Grind/Arith/Linear/`.

### Impact

Add :+1: to issues you consider important. If others are impacted by this issue, please ask
them to add :+1: to it.

Found by an automated differential tester ([ephyra](https://github.com/datadog-labs/ephyra)); this report was drafted with AI assistance.
