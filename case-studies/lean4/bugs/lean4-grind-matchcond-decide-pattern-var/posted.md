### Prerequisites

* [x] Check that your issue is not already filed: searched for `grind match kernel`, `mkMatchCondProof`, `grind decide pattern variable`, `grind kernel application type mismatch decide` and `grind matchCond`. The nearest are #13773 and its fix #14020 (same function, different bugs), #15697 and #15541/#15604/#15605.
* [x] Reduce the issue to a minimal, self-contained, reproducible test case. Avoid dependencies to Mathlib or Batteries.
* [x] Test your test case against the latest nightly release.

### Description

`grind` should not produce proofs that the kernel rejects. On a `match` with overlapping
patterns, when the discriminant has a literal where the earlier pattern has a variable,
`grind [f]` closes the goal with a proof containing `decide ¬3 = a`. Here `a` is a pattern
variable, so the kernel rejects the proof with `(kernel) application type mismatch`.

### Steps to Reproduce

```lean
def p (s : Nat × Bool) : Nat := match s with | (_, true) => 0 | _ => 1
theorem t : p (3, false) = 1 := by grind [p]
```

```
error: (kernel) application type mismatch
  id (Eq.refl true)
argument has type
  true = true
but function has type
  (decide ¬3 = fst✝) = true → (decide ¬3 = fst✝) = true
```

The same happens with a literal in the discriminant:

```lean
def f0 (x : Nat) := match (x, 1) with | (a, 0) => a | _ => 7
theorem t : f0 3 = 7 := by grind [f0]
-- (kernel) application type mismatch ... (decide ¬3 = a✝) = true → ...
```

It also happens with a `UInt8` field:

```lean
def f2 (x : Nat) (y : UInt8) : Nat := match (x, y) with | (a, 0) => a | _ => 7
example : f2 3 1 = 7 := by grind [f2]
-- (kernel) application type mismatch
```

**Expected behavior:** `grind` proves the goals. The second arm applies because the first
arm's literal field (`true`, `0`) does not match. Or `grind` fails without producing an
ill-typed proof.

**Actual behavior:** the proof is rejected by the kernel. The refuted equation is the one
between the literal `3` and the pattern variable, not the one between the two literals.

### Versions

```
4.37.0-nightly-2026-10-30   p: kernel error   f0: kernel error   f2: kernel error
master b21ac7949a            p: kernel error   f0: kernel error   f2: kernel error
4.35.0                       p: kernel error   f0: kernel error   f2: kernel error
4.33.0                       p: kernel error   f0: kernel error
4.29.0, 4.28.0, 4.22.0       p: kernel error   f0: accepted
```

These are on macOS arm64 (official toolchains via elan).

### Impact

Add :+1: to issues you consider important. If others are impacted by this issue, please ask
them to add :+1: to it.

Found by an automated differential tester ([ephyra](https://github.com/datadog-labs/ephyra)); this report was drafted with AI assistance.
