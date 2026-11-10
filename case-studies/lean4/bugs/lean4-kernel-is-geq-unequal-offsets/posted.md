### Prerequisites

* [x] Check that your issue is not already filed: searched for `is_geq`, `universe level of type_of`, `is too big for the corresponding inductive datatype` and `Level.geq`. Related: #2675 (closed, a different `imax` case) and #2689 (made `Level.geq` match the kernel for `max u v ≥ imax u v`).
* [x] Reduce the issue to a minimal, self-contained, reproducible test case. Avoid dependencies to Mathlib or Batteries.
* [x] Test your test case against the latest nightly release.

### Description

The kernel rejects a structure whose field universe is small enough:

```lean
universe u
structure J : Type (u+1) where
  x : Sort (imax (u+1) u)
```

```
lean Geq.lean
Geq.lean:2:0: error: (kernel) universe level of type_of(arg #1) of 'J.mk' is too big for the corresponding inductive datatype
```

The field lives in `Sort (imax (u+1) u + 1)` and the structure in `Sort (u+2)`. For every `u`, `imax (u+1) u + 1 ≤ u + 2`: when `u = 0` the left side is `1`, otherwise it is `u + 2`. The error comes from the kernel only. The elaborator's check agrees that the level fits:

```lean
import Lean
open Lean
#eval (Level.succ (.succ (.param `u))).geq (.succ (.imax (.succ (.param `u)) (.param `u)))
-- true
```

`inductive J (β : Type (imax (u+1) u)) : Type (u+1) | mk : β → J β` fails the same way. With the larger field `x : Sort (u+1)`, the structure is accepted.

`is_geq_core` in `src/kernel/level.cpp` removes `succ` offsets only when both sides have the same offset:

```cpp
if (p1.first == p2.first || is_zero(p2.first))
    return p1.second >= p2.second;
if (p1.second == p2.second && p1.second > 0)
    return is_geq(p1.first, p2.first);
return false;
```

Here the offsets are 2 (`u+2`) and 1 (`imax (u+1) u + 1`), so it returns `false`. `Level.geq` has `| succ u, succ v => go u v`, and removing one `succ` from each side gives `u+1 ≥ imax (u+1) u`, which `is_geq` proves.

**Expected behavior:** the file checks, as it does with `x : Sort (u+1)`.

**Actual behavior:** the kernel rejects the constructor.

### Versions

```
4.37.0-nightly-2026-11-03   fails
master b21ac7949a            fails
4.36.0-rc2                   fails
4.35.0                       fails
4.22.0                       fails
```

These are on macOS arm64 (official toolchains via elan).

### Impact

Add :+1: to issues you consider important. If others are impacted by this issue, please ask
them to add :+1: to it.

Found by an automated differential tester ([ephyra](https://github.com/datadog-labs/ephyra)); this report was drafted with AI assistance.
