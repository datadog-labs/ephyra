## Program (`prog.ml`)

```ocaml
type t4 = #(int * int * int * int)
type t16 = #(t4 * t4 * t4 * t4)
type t64 = #(t16 * t16 * t16 * t16)
type t256 = #(t64 * t64 * t64 * t64)
let f (_ : t256) (y : int) = y
let () = print_endline "ok"
```

## Results

OxCaml trunk 0d3c93580d (`5.4.0+ox`), amd64 Linux, configured with `--enable-flambda-invariants`.

```
ocamlc.opt prog.ml -o p.byte && ./p.byte
# ok

ocamlopt.opt prog.ml -o p.exe
# expected: links; ./p.exe prints ok
# oxcaml:   Exception whilst emitting instruction:
#           anon:V/540[%r10] := alloc 2080
#           Fatal error: exception File "backend/amd64/emit.ml", line 2356, characters 4-10: Assertion failed

# the same with -O3 and with -Oclassic; ocamlopt.opt -c prog.ml succeeds
```

The size matters: a flat `#(int * ... * int)` first parameter of 253 components links and runs, and one of
254 components fails the same way.

Found by an automated differential tester ([ephyra](https://github.com/datadog-labs/ephyra))

