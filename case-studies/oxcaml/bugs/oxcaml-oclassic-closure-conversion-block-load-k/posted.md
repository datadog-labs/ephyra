# `ocamlopt -Oclassic` crashes on a `float#` field read in an unreachable match arm

Posted as https://github.com/oxcaml/oxcaml/issues/7683 on 2026-10-22. The text below is the issue as posted.

---

## Program (`repro/prog.ml`, LLM-shrunk from `original/`)

```ocaml
type v = Q of float# | S of float
let f (p : float#) =
  let _ = match S 0.0 with Q f -> Stdlib_upstream_compatible.Float_u.floor f | _ -> p in
  ()
```

## Results

OxCaml trunk aadb3804 (`5.4.0+ox`), amd64 Linux. `main` has 6 newer commits; none touches
`closure_conversion.ml` or the emitter.

```
ocamlopt -Oclassic -w -a -I +stdlib_upstream_compatible stdlib_upstream_compatible.cmxa prog.ml -o p.exe
# expected: compiles
# oxcaml:   >> Fatal error: Illegal move between registers of differing types (...), exit 2

# same command without -Oclassic, or with -O3
# oxcaml:   compiles
```
