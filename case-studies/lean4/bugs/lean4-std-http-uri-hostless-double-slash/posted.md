### Prerequisites

* [x] Check that your issue is not already filed: searched for `Std.Http URI toString`, `URI authority path`, `URI round trip`, `URI.Builder` and `Std.Http URI parse`. Related: #15246, #16021, #16058 (other URI round trips).
* [x] Reduce the issue to a minimal, self-contained, reproducible test case. Avoid dependencies to Mathlib or Batteries.
* [x] Test your test case against the latest nightly release.

### Description

A URI built with no host and path segments `#["", "a"]` (path `//a`) prints as `https://a`, and that text parses back as a different URI, with host `a` and an empty path:

```lean
import Std.Http
open Std.Http

def main : IO Unit := do
  -- no host; path segments ["", "a"]
  let u := (URI.Builder.empty.setScheme! "https").setPath #["", "a"] |>.build
  IO.println s!"built:  authority={u.authority.isSome} path={u.path.toDecodedSegments}"
  IO.println s!"text:   {u}"
  match URI.parse? (toString u) with
  | some p => IO.println s!"parsed: authority={p.authority.map (toString ·.host)} path={p.path.toDecodedSegments}"
  | none   => IO.println "parsed: none"
```

```
lean --run Hostless.lean
built:  authority=false path=#[, a]
text:   https://a
parsed: authority=(some a) path=#[]
```

The module docstring of `src/Std/Http/Data/URI/Basic.lean` says the URI structure is "aligned with RFC 3986". RFC 3986 §3.3 says: "If a URI does not contain an authority component, then the path cannot begin with two slash characters ("//")". `ToString URI` adds `//` only when there is an authority, and `ToString Path` prints the segments joined by `/` after a leading `/`. So a path that starts with an empty segment prints as if it were an authority.

**Expected behavior:** the printed text parses back to the same URI (no authority, the same path), or `build` rejects or normalizes such a path.

**Actual behavior:** the text names a different resource (host `a`).

### Versions

```
4.37.0-nightly-2026-11-04   fails
4.36.0-rc2                   fails
4.35.0                       fails
```

These are on macOS arm64 (official toolchains via elan). The URI commits on master since the nightly (#16079, #16265, #15892, #15254) do not change how URIs and paths are printed.

### Impact

Add :+1: to issues you consider important. If others are impacted by this issue, please ask
them to add :+1: to it.

Found by an automated differential tester ([ephyra](https://github.com/datadog-labs/ephyra)); this report was drafted with AI assistance.
