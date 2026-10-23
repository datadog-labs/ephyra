# `ocamlopt -Oclassic` crashes on a division by constant zero inside a `try`

Posted as https://github.com/oxcaml/oxcaml/issues/7668 on 2026-10-21. The text below is the issue as posted.

---

## Program (`repro/prog.ml`)

```ocaml
let f y =
  let r = ref 0 in
  r := y;
  try y mod 0 with Not_found -> !r
```

## Results

OxCaml trunk aadb3804 (`ocamlopt -version`: `5.4.0+ox`), built with `--enable-flambda-invariants`,
amd64 Linux. `main` has 6 newer commits; none touches `to_cmm`, `cmm_helpers`, `cfg_selectgen` or
`lambda_to_flambda`.

```
ocamlopt -Oclassic -c prog.ml
# expected: compiles
# oxcaml:   Fatal error: exception Invalid_argument("Array.iter2: arrays must have the same length"), exit 2

ocamlopt -c prog.ml          # default level
ocamlopt -O2 -c prog.ml
ocamlopt -O3 -c prog.ml
# oxcaml:   compiles
```
