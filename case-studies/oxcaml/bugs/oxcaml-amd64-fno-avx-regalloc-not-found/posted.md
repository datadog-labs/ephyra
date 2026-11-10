# Posted: https://github.com/oxcaml/oxcaml/issues/7919

Filed by alpaylan, 2026-11-06. Text as posted:

**amd64: `ocamlopt -fno-avx` fails with `Fatal error: exception Not_found` on a store to a float32 Bigarray**

OxCaml main 9d845db9ba (2026-11-05) and 0d3c93580d;  amd64 Linux, `./configure --enable-flambda-invariants`.

## Program (`prog.ml`)

```ocaml
let a = Bigarray.(Array1.create float32 c_layout 1)
let f x = a.{0} <- x; x +. 1.
let () = let y = f 2.5 in Printf.printf "%g %g\n" y a.{0}
```

## Results

```
ocamlopt.opt prog.ml -o p.exe && ./p.exe
# 3.5 2.5

ocamlopt.opt -fno-avx prog.ml -o p.exe
# Fatal error: exception Not_found        (exit 2)

# -O3 -fno-avx:                     Fatal error: exception Not_found
# -Oclassic -fno-avx:               3.5 2.5
# -fno-avx -regalloc ls:            3.5 2.5
```
