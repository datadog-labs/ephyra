### Prerequisites

* [x] Check that your issue is not already filed: searched for `toPolyC`, `maxChar`, `characteristic 1` and `backward.grind.normalizer`. Related: #15615 (the same goal shape in the `grind` ring solver, fixed by #15850, which is in the failing builds).
* [x] Reduce the issue to a minimal, self-contained, reproducible test case. Avoid dependencies to Mathlib or Batteries.
* [x] Test your test case against the latest nightly release.

### Description

With the `Sym.simp`-based normalizer and `maxChar := 1`, `grind` builds a proof of a true goal in characteristic 1 that the kernel rejects:

```lean
set_option backward.grind.normalizer false in
example (x : Fin 1) : (0 : Fin 1) = x ^ 2 := by grind (maxChar := 1)
```

```
lean CharOne.lean
CharOne.lean:2:48: error: (kernel) application type mismatch
  Lean.Grind.CommRing.eq_true_of_toPolyC_eq (Lean.RArray.leaf x) false false
    (Lean.Grind.CommRing.Expr.num (Int.ofNat 0)) ((Lean.Grind.CommRing.Expr.var 0).pow 2)
    (Lean.Grind.CommRing.Poly.num (Int.ofNat 0)) (Eq.refl (Lean.Grind.CommRing.Poly.num (Int.ofNat 0)))
    (Eq.refl (Lean.Grind.CommRing.Poly.num (Int.ofNat 0)))
argument has type
  Lean.Grind.CommRing.Poly.num (Int.ofNat 0) = Lean.Grind.CommRing.Poly.num (Int.ofNat 0)
but function has type
  Lean.Grind.CommRing.Poly.num (Int.ofNat 0) =
      Lean.Grind.CommRing.Expr.toPolyXC_k false ((Lean.Grind.CommRing.Expr.var 0).pow 2) 1 →
  ...
```

`x : BitVec 0` with `(0 : BitVec 0) = x ^ 2` fails the same way. These variants are proved:
`x * x` instead of `x ^ 2`, `x = x ^ 2`, `maxChar := 0`, and the default normalizer (with or
without `maxChar := 1`).

**Expected behavior:** `grind` proves the goal (or fails), as it does with the default normalizer.

**Actual behavior:** the kernel rejects the proof.

### Versions

```
4.37.0-nightly-2026-11-03   fails
4.37.0-nightly-2026-10-30   fails
master b21ac7949a            fails
4.36.0-rc2                   `maxChar` is not a `grind` option
4.35.0                       `backward.grind.normalizer` does not exist
```

These are on macOS arm64 (official toolchains via elan). The proof comes from `constRelProof?` in `src/Lean/Meta/Sym/Arith/Norm.lean` (#15988).

### Impact

Add :+1: to issues you consider important. If others are impacted by this issue, please ask
them to add :+1: to it.

Found by an automated differential tester ([ephyra](https://github.com/datadog-labs/ephyra)); this report was drafted with AI assistance.
