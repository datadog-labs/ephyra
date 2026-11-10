import Lean
open Lean

-- Import `B` (which imports `C`) twice in one process.
unsafe def main : IO Unit := do
  initSearchPath (← findSysroot)
  for i in [0:2] do
    withImportModules #[{ module := `B }] {} fun env =>
      IO.println s!"import {i}: b found = {(env.find? `b).isSome}"
