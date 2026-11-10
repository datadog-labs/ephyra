### Prerequisites

* [x] Check that your issue is not already filed: searched for `withImportModules segfault`, `findOLean`, `freeRegions` and `oleanRootCache`. Related: #15291 (added the cache).
* [x] Reduce the issue to a minimal, self-contained, reproducible test case. Avoid dependencies to Mathlib or Batteries.
* [x] Test your test case against the latest nightly release.

### Description

Importing a module twice with `withImportModules` in one process segfaults when that module imports another one:

```lean
-- C.lean
def c : Nat := 1
```

```lean
-- B.lean
import C
def b : Nat := c
```

```lean
-- Main.lean
import Lean
open Lean

unsafe def main : IO Unit := do
  initSearchPath (← findSysroot)
  for i in [0:2] do
    withImportModules #[{ module := `B }] {} fun env =>
      IO.println s!"import {i}: b found = {(env.find? `b).isSome}"
```

```sh
export LEAN_PATH=$PWD
lean -o C.olean C.lean
lean -o B.olean B.lean
lean --run Main.lean; echo "exit $?"
```

```
# 4.37.0-nightly-2026-11-04 and 4.36.0-rc2:
exit 139   (segmentation fault; the first line of output is lost)

# 4.35.0:
import 0: b found = true
import 1: b found = true
exit 0
```

`findOLean` (`src/Lean/Util/Path.lean`) memoizes each package's root in the process-global `oleanRootCacheRef`, added in #15291. The cached `Name` can live in the compacted region of an imported `.olean`, here the name `C` from `B.olean`'s import list. `withImportModules` then frees those regions (`Environment.freeRegions`, whose docstring says "No live references to imported objects may exist at the time of invocation"). The next `findOLean` compares against the freed name.

**Expected behavior:** both imports succeed, as on 4.35.0.

**Actual behavior:** the second import segfaults.

### Versions

```
4.37.0-nightly-2026-11-04   segfault
4.36.0-rc2                   segfault
4.35.0                       works
```

These are on macOS arm64 (official toolchains via elan). It also fails on Linux x86_64 builds of master. #15291 (62e2afd1a5) is in 4.36.0-rc1 and not in 4.35.0.

### Impact

Add :+1: to issues you consider important. If others are impacted by this issue, please ask
them to add :+1: to it.

Found by an automated differential tester ([ephyra](https://github.com/datadog-labs/ephyra)); this report was drafted with AI assistance.
