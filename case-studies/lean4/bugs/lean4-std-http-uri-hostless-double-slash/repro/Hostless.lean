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
