# non-exhaustive integer match crashes instead of raising Match_failure

Posted as https://github.com/ocaml/ocaml/issues/15185 on 2026-10-21. The text below is the issue as posted.

---

## Program (`repro/m.ml`)

```ocaml
let f x y = match x, y with
  | _, true -> 0
  | (0 | 2), _ -> 1
  | 3, true -> 2
  | -1, _ -> 3
let () = print_int (f 4 false)
```

No case matches `(4, false)`.

## Contract

Reference manual, pattern matching (`manual/src/refman/expr.etex`, lines 637-638 at trunk 3c41bbf8):
"If none of the patterns match the value of expr, the exception Match_failure is raised."

## Results

Checked 2026-10-21 in the official Docker images `ocaml/opam:ubuntu-24.04-ocaml-5.4` (5.4.1) and
`ocaml/opam:ubuntu-24.04-ocaml-4.14` (4.14.4), amd64 Linux:

```
ocamlc -w -a m.ml -o m.byte && ./m.byte
# expected: Fatal error: exception Match_failure("m.ml", 1, 12)
# 5.4.1:    Illegal instruction (core dumped), exit 132
# 4.14.4:   Segmentation fault (core dumped), exit 139

ocamlopt -w -a m.ml -o m.exe && ./m.exe
# expected: Fatal error: exception Match_failure("m.ml", 1, 12)
# 5.4.1:    Segmentation fault (core dumped), exit 139
# 4.14.4:   Segmentation fault (core dumped), exit 139
```
