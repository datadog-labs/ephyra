### Prerequisites

* [x] Check that your issue is not already filed: searched for `eq_of_dvd_ediv`, `elimNonUnit`, `lia kernel`, `cutsat` and `application type mismatch`. Related: #15622 (a `lia` panic with variables created during case-split search, fixed by #15831).
* [x] Reduce the issue to a minimal, self-contained, reproducible test case. Avoid dependencies to Mathlib or Batteries.
* [x] Test your test case against the latest nightly release.

### Description

`lia` and `grind` build a proof of the goal below without an elaboration error, and the kernel rejects it:

```lean
-- `4 * x % 3` is `x % 3`, so the goal is `p → p`.
example (x : Int) : x % 3 ≤ 2 * x % 3 → 4 * x % 3 ≤ 2 * x % 3 := by lia

example (x : Int) : x % 3 ≤ 2 * x % 3 → 4 * x % 3 ≤ 2 * x % 3 := by grind
```

```
lean Bug.lean
Bug.lean:2:68: error: (kernel) application type mismatch
  Int.Internal.Linear.eq_of_dvd_ediv ctx 3
    ((Int.Internal.Linear.Expr.mulL (-1) (Int.Internal.Linear.Expr.var 0)).add (Int.Internal.Linear.Expr.num 0))
    (Int.Internal.Linear.Poly.add (-1) 0 (Int.Internal.Linear.Poly.num 0))
    (Int.Internal.Linear.Poly.add (-3) 4 (Int.Internal.Linear.Poly.add (-1) 0 (Int.Internal.Linear.Poly.num 0))) 4
    (Eq.refl ((-1 * x + 0) / 3))
argument has type
  (-1 * x + 0) / 3 = (-1 * x + 0) / 3
but function has type
  Int.Internal.Linear.Var.denote ctx 4 =
      Int.Internal.Linear.Expr.denote_k ctx
          ((Int.Internal.Linear.Expr.mulL (-1) (Int.Internal.Linear.Expr.var 0)).add (Int.Internal.Linear.Expr.num 0)) /
        3 →
  ...
Bug.lean:4:68: error: (kernel) application type mismatch
  (the same error for the `grind` example)
```

The proof refers to variable 4, whose denotation in `ctx` is not `(-1 * x + 0) / 3`.

**Expected behavior:** both examples check, as they do on v4.36.0-rc2.

**Actual behavior:** the kernel rejects both proofs.

### Versions

```
4.37.0-nightly-2026-11-03   fails
4.37.0-nightly-2026-10-30   fails
master b21ac7949a            fails
4.36.0-rc2                   works
4.35.0                       works
```

These are on macOS arm64 (official toolchains via elan). The failing builds contain #15838 (`EqCnstr.elimNonUnit`, which adds variables during the search); 4.36.0-rc2 does not.

### Impact

Add :+1: to issues you consider important. If others are impacted by this issue, please ask
them to add :+1: to it.

Found by an automated differential tester ([ephyra](https://github.com/datadog-labs/ephyra)); this report was drafted with AI assistance.
