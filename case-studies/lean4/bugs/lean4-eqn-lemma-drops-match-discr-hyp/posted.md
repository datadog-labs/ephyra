### Prerequisites

* [x] Check that your issue is not already filed: searched for `failed to generate equational theorem`, `equational theorem match discriminant`, `equation lemma heq` and `isIrrelevant`. Related: #15735 (a `match` on the decreasing argument and a non-variable value, with structural recursion and a bind), #5667, #10431.
* [x] Reduce the issue to a minimal, self-contained, reproducible test case. Avoid dependencies to Mathlib or Batteries.
* [x] Test your test case against the latest nightly release.

### Description

Lean accepts this definition but cannot generate its equation lemmas:

```lean
def f (a : Nat) : Nat :=
  match id a, a with
  | 0, _ => 1
  | _+1, 0 => 2
  | _, _+1 => 3

#check @f.eq_1
```

```
lean EqnDiscr.lean
EqnDiscr.lean:7:8: error: Failed to realize constant f.eq_1:
  failed to generate equational theorem for `f`
  case h_1
  x a : Nat
  heq✝ : id 0 = 0
  ⊢ 1 = 2
EqnDiscr.lean:7:8: error(lean.unknownIdentifier): Unknown identifier `f.eq_1`
```

For the second alternative, the statement Lean computes is `f 0 = 2` without the hypothesis `id 0 = n.succ` from the split, and that statement is false (`f 0` is `1`). `simp [f]` and `rw [f]` fail the same way (`unfold f` works). A closed discriminant gives the same failure:

```lean
def g (a : Nat) : Nat :=
  match 7 - 0, a with
  | 0, _ => 1
  | _+1, 0 => 2
  | _, _+1 => 3
example : g 0 = 2 := by simp [g]
-- error: failed to generate equational theorem for `g`
```

With `match a, a with` the lemmas are generated.

**Expected behavior:** the equation lemmas are generated, with the hypothesis kept, for example `f.eq_2 (n : Nat) : id 0 = n.succ → f 0 = 2`.

**Actual behavior:** generation fails, so `f.eq_1` does not exist and `simp [f]` fails.

### Versions

```
4.37.0-nightly-2026-11-04   fails
master 92016b6068            fails
4.35.0                       fails
4.22.0                       fails (times out instead)
```

These are on macOS arm64 (official toolchains via elan); master was built on Linux.

### Impact

Add :+1: to issues you consider important. If others are impacted by this issue, please ask
them to add :+1: to it.

Found by an automated differential tester ([ephyra](https://github.com/datadog-labs/ephyra)); this report was drafted with AI assistance.
