### Prerequisites

* [x] Check that your issue is not already filed: searched for `match Float literal`, `Float pattern match literal` and `match_float`. Related: #14181 (the feature) and #15823 (the same literal-versus-value confusion, in `grind`).
* [x] Reduce the issue to a minimal, self-contained, reproducible test case. Avoid dependencies to Mathlib or Batteries.
* [x] Test your test case against the latest nightly release.

### Description

Float literal patterns are matched by the literal as written, not by its value. Two
literals that denote the same float are treated as different patterns. So an alternative
whose pattern equals the scrutinee is skipped when an earlier alternative used the other
spelling:

```lean
def f (s : Float32) (n : Nat) : Nat :=
  match s, n with
  | 16777216, 0 => 6
  | 16777217, _ => 7
  | _, _ => 0

#eval f 16777217 1                       -- 0
example : f 16777217 1 = 7 := by decide  -- error: `decide` proved that the proposition `f 16777217 1 = 7` is false
```

As a `Float32`, `16777217` is `16777216`. So for `(16777217, 1)` the first alternative fails
on `n` and the second, `16777217, _`, matches: the result should be 7. The same happens with
`Float` and `5`/`5.0`, or `1`/`1.0`, in the two first columns.

`tests/elab/match_float.lean` (added with the feature in #14181) says: "Matching uses the
`DecidableEq` instance (propositional, bit-pattern equality)". Under that instance,
`(16777216 : Float32) = 16777217` holds (`by decide` and `rfl` both prove it).

The elaborated matcher is wrong, so `#eval`, compiled code and the kernel all agree on 0.

**Expected behavior:** `f 16777217 1 = 7`.

**Actual behavior:** `f 16777217 1 = 0`.

### Versions

```
4.37.0-nightly-2026-10-30   0
master b21ac7949a            0
4.35.0                       0
4.33.0                       0
4.29.0                       float literal patterns not supported ("Dependent elimination failed")
```

These are on macOS arm64 (official toolchains via elan).

### Additional Information

In `src/Lean/Meta/Match/Match.lean`, the value patterns of a column are collected with
`values.contains v` (line 656) and an alternative is kept for a value when `v == value`
(line 691). Both compare the literal expressions, which is right for `Nat`, `String` or `Char`
literals, where distinct literals are distinct values. It is wrong for floats.

A fix with a test is available.

### Impact

Add :+1: to issues you consider important. If others are impacted by this issue, please ask
them to add :+1: to it.

Found by an automated differential tester ([ephyra](https://github.com/datadog-labs/ephyra)); this report was drafted with AI assistance.
