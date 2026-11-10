### Prerequisites

* [x] Check that your issue is not already filed: searched for `leanir Unknown constant`, `postponeCompile`, `leanir ir.sig` and `leanir private`. Related: #13103 and #15583 (the reverse direction: plain modules importing postponed ones).
* [x] Reduce the issue to a minimal, self-contained, reproducible test case. Avoid dependencies to Mathlib or Batteries.
* [x] Test your test case against the latest nightly release.

### Description

A Lake package that enables separate code generation cannot build a module that uses
`Array.map`:

```lean
-- lakefile.lean
import Lake
open Lake DSL

package l where
  leanOptions := #[⟨`compiler.postponeCompile, true⟩]

@[default_target]
lean_lib L
```

```lean
-- L.lean
module
public import L.A
```

```lean
-- L/A.lean
module
public section
def f0 (xs : Array Nat) : Array String := xs.map toString
```

```sh
lake build
# ✖ [3/6] Building L.A:irArts
# .lake/build/lib/lean/L/A.ir:0:0: error: Failed to compile `f0`
#   Unknown constant `_private.Init.Data.Array.Basic.0.Array.mapMUnsafe.map`
# error: external command '.../bin/leanir' exited with code 1
```

Without `compiler.postponeCompile`, the same package builds.

`Array.map` is `@[inline]`, and inlining it reaches the private helper `Array.mapMUnsafe.map`,
whose IR is public. `Init` is compiled without `postponeCompile`, so plain `lean` writes a
placeholder `.ir.sig` for it, to be filled by `leanir`. When `leanir` compiles the postponed
module, it reads that placeholder for the import instead of the exported module data. The
placeholder has no `extraConstNames`, so the private helper is never found.

**Expected behavior:** `lake build` succeeds, as it does without `compiler.postponeCompile`.

**Actual behavior:** `leanir` fails with `Unknown constant _private...`.

### Versions

```
4.37.0-nightly-2026-10-30   fails (lake build, and lean + leanir directly)
master b21ac7949a            fails (lean + leanir directly)
4.35.0                       fails (lake build, and lean + leanir directly)
4.33.0                       fails (lean + leanir directly)
```

These are on macOS arm64 (official toolchains via elan).

### Impact

Add :+1: to issues you consider important. If others are impacted by this issue, please ask
them to add :+1: to it.

Found by an automated differential tester ([ephyra](https://github.com/datadog-labs/ephyra)); this report was drafted with AI assistance.
