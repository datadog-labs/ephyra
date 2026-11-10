# Posted: https://github.com/oxcaml/oxcaml/issues/7889

Filed by alpaylan, 2026-11-05. Text as posted:

**amd64: `Atomic.compare_and_set` on an `int Atomic.t` succeeds after an earlier failed CAS with the same expected value**

## Program (`prog.ml`)

```ocaml
(* r holds 5, so both compare-and-sets expect 1 and must fail *)
let[@inline never] cas_twice (seen : int) r (v : int) =
  ignore (Atomic.compare_and_set r seen v);
  Atomic.compare_and_set r seen v

let () =
  let r = Atomic.make 5 in
  let swapped = cas_twice 1 r 7 in
  Printf.printf "%b %d\n" swapped (Atomic.get r)
```

## Results

OxCaml trunk 0d3c93580d (`5.4.0+ox`), amd64 Linux, configured with `--enable-flambda-invariants`.

```
ocamlopt.opt prog.ml -o p.exe && ./p.exe
# expected: false 5
# oxcaml:   true 7

# -O3, -O2, -O3 -regalloc irc:            true 7
# -Oclassic, -O3 -regalloc ls, bytecode:  false 5
# upstream OCaml 5.4.0 ocamlopt:          false 5
```
