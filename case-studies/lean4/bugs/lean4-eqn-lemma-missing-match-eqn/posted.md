### Prerequisites

* [x] Check that your issue is not already filed: searched for `match_1.eq_1`, `unknown constant eq_1`, `equation lemma unknown constant` and `realizeValue`. Related: #15987 (a proof referring to a missing equation lemma, via `cbv` and kernel deep recursion).
* [x] Reduce the issue to a minimal, self-contained, reproducible test case. Avoid dependencies to Mathlib or Batteries.
* [x] Test your test case against the latest nightly release.

### Description

When a definition's body has a `match` on a literal under a dependent argument, its equation lemma `f.eq_1` refers to the match equation `f.match_1.eq_1`, which is never added to the environment:

```lean
def f : Nat :=
  Nat.divExact (2 * (match 0 with | 0 => 1 | _ => 3)) _ (Nat.dvd_mul_left _ 2)

theorem t : f = 2 := by simp [f]
```

```
lean EqnMatch.lean
EqnMatch.lean:4:30: error: (kernel) unknown constant '_private.EqnMatch.0.f.match_1.eq_1'
```

`#check f.eq_1` fails with the same kernel error. These variants are accepted:

```lean
def f : Nat := 2 * (match 0 with | 0 => 1 | _ => 3)
theorem t : f = 2 := by simp [f]

def g : Nat :=
  Nat.divExact (2 * (match true with | true => 1 | false => 3)) _ (Nat.dvd_mul_left _ 2)
theorem tg : g = 2 := by simp [g]
```

**Expected behavior:** the file checks, as on 4.35.0.

**Actual behavior:** the kernel rejects the equation lemma, so `simp [f]` fails.

### Versions

```
4.37.0-nightly-2026-11-03   fails
4.36.0-rc2                   fails
4.35.0                       works
```

These are on macOS arm64 (official toolchains via elan). It also fails on a Linux build of master c029d915d1 (2026-11-04), which includes #16208. #15499 (which moved the computation of equation lemma types into `realizeValue`) is in 4.36.0-rc1 and not in 4.35.0.

### Impact

Add :+1: to issues you consider important. If others are impacted by this issue, please ask
them to add :+1: to it.

Found by an automated differential tester ([ephyra](https://github.com/datadog-labs/ephyra)); this report was drafted with AI assistance.
