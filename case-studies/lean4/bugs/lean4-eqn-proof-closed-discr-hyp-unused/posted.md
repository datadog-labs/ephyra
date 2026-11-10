### Prerequisites

* [x] Check that your issue is not already filed: searched for `failed to generate equational theorem`, `equational theorem match discriminant` and `closed discriminant`. Related: #16259 (same kind of `match`, but there the hypothesis is dropped from the statement), #7841, #15735, #10431.
* [x] Reduce the issue to a minimal, self-contained, reproducible test case. Avoid dependencies to Mathlib or Batteries.
* [x] Test your test case against the latest nightly release.

### Description

Lean accepts this definition and states a true equation lemma for it, but cannot prove that lemma:

```lean
def f (a : Nat) : Nat :=
  match 1 - a, a with
  | n+1, 0 => n
  | _, _ => 0

#check @f.eq_1
example : f 0 = 0 := by simp [f]
```

```
Eqn.lean:6:8: error: Failed to realize constant f.eq_1:
  failed to generate equational theorem for `f`
  n : Nat
  h : 1 - 0 = n.succ
  ⊢ 0 = n
Eqn.lean:6:8: error(lean.unknownIdentifier): Unknown identifier `f.eq_1`
Eqn.lean:7:30: error: failed to generate equational theorem for `f`
...
Eqn.lean:7:24: error: `simp` made no progress
```

The hypothesis `h : 1 - 0 = n.succ` is in the goal and gives `n = 0`, but the proof doesn't use it. `example : f 0 = 0 := rfl` and `by unfold f; rfl` are both accepted. Unlike #16259, the generated statement here is correct; only its proof fails. `5 - a` fails the same way, while `match id 3, a with ...` works (`h.eq_1 : ∀ n, id 3 = n.succ → h 0 = n`).

**Expected behavior:** `f.eq_1 : ∀ (n : Nat), 1 - 0 = n.succ → f 0 = n` is generated and `simp [f]` proves `f 0 = 0`.

**Actual behavior:** the errors above.

### Versions

```
4.37.0-nightly-2026-11-05   failed to generate equational theorem
4.35.0                       failed to generate equational theorem (hypothesis named heq)
4.22.0                       (deterministic) timeout at `whnf`
```

These are on macOS arm64 (official toolchains via elan), and Linux x86_64 gives the same results. Since the nightly, the only change on master to `src/Lean/Elab/PreDefinition/Eqns.lean` is #16275, which is about over-applied matchers.

### Impact

Add :+1: to issues you consider important. If others are impacted by this issue, please ask
them to add :+1: to it.

Found by an automated differential tester ([ephyra](https://github.com/datadog-labs/ephyra)); this report was drafted with AI assistance.
