# Posted: https://github.com/oxcaml/oxcaml/issues/7918

Filed by alpaylan, 2026-11-06. Text as posted:

**Bytecode: `Effect.perform` segfaults `ocamlrun` when a memprof callback grows the fiber stack**

Twin of the OCaml problem: https://github.com/ocaml/ocaml/issues/15266

OxCaml main 9d845db9ba (2026-11-05) and 0d3c93580d;  amd64 Linux, `./configure --enable-flambda-invariants`.

## Program (`repro/perform.ml`)

```ocaml
type _ Effect.t += E : unit Effect.t
let rec deep n = if n = 0 then 0 else 1 + deep (n - 1)
let () =
  let _ = Gc.Memprof.start ~sampling_rate:1.0
    { Gc.Memprof.null_tracker with alloc_minor = fun _ -> ignore (deep 1000); None } in
  Effect.Deep.try_with Effect.perform E
    { effc = fun (type a) (e : a Effect.t) -> match e with
        | E -> Some (fun (k : (a, _) Effect.Deep.continuation) -> Effect.Deep.continue k ())
        | _ -> None };
  print_endline "ok"
```

## Results

```
ocamlopt.opt -alert -all perform.ml -o perform.exe && ./perform.exe
# ok

ocamlc.opt -alert -all perform.ml -o perform.byte && ocamlrun ./perform.byte
# Segmentation fault (exit status 139), on every run
```
