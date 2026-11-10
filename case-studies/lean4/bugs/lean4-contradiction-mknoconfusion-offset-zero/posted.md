### Prerequisites

* [x] Check that your issue is not already filed: searched for `mkNoConfusion No manifest constructors`, `No manifest constructors`, `contradiction internal error Nat` and `contradiction 0 + 0`. Related: #6515 and #6731 (`contradiction` on types without `noConfusion`; `Nat` has one).
* [x] Reduce the issue to a minimal, self-contained, reproducible test case. Avoid dependencies to Mathlib or Batteries.
* [x] Test your test case against the latest nightly release.

### Description

```lean
example (h : 0 + 0 = 1) : False := by contradiction
```

```
lean Zero.lean
Zero.lean:1:38: error: mkNoConfusion: No manifest constructors in 0 + 0 = 1
```

`h : 0 + 0 = n + 1` and `h : n + 1 = 0 + 0` (`n : Nat`) fail the same way. The docstring's own example, `h : 2 + 2 = 3`, works, and so does `h : 0 + 1 = 0`.

The `contradiction` docstring lists "Injectivity of constructors" and "Decidable false proposition" among the cases it closes, and `0 + 0 = 1` is both. `contradiction` recognizes `0 + 0` as `Nat.zero` and `1` as `Nat.succ` (`matchConstructorApp?`) and calls `mkNoConfusion`. `mkNoConfusion` then fails to see a constructor in `0 + 0`: it reads `0 + 0` as an offset with `k = 0` (`constructorApp'?` in `CtorRecognizer.lean`), and that branch returns `none`.

**Expected behavior:** the goal is closed, as on 4.22.0.

**Actual behavior:** an internal error.

### Versions

```
4.37.0-nightly-2026-11-05   internal error
4.35.0, 4.34.1, 4.33.0      internal error
4.30.0-rc2, 4.29.0, 4.28.0  internal error
4.22.0                      works
```

These are on macOS arm64 (official toolchains via elan). Commits in that range that touch `mkNoConfusion` include #10318 ("use per-constructor noConfusion in `mkNoConfusion`") and #10300. I did not bisect.

### Impact

Add :+1: to issues you consider important. If others are impacted by this issue, please ask
them to add :+1: to it.

Found by an automated differential tester ([ephyra](https://github.com/datadog-labs/ephyra)); this report was drafted with AI assistance.
