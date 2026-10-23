# `-Oclassic -flambda2-expert-inline-effects-in-cmm`: a call used as the argument of another call inside a `try` crashes ocamlopt with `Invalid_argument("Array.iter2 ...")`

Posted as https://github.com/oxcaml/oxcaml/issues/7705 on 2026-10-23. The text below is the issue as posted.

---

Same crash site as #7668, but different problem; #7669 does not fix it (checked on aadb3804 + #7669).

## Program (`prog.ml`)

```ocaml
let f g h x = let r = ref x in try g (h x) with _ -> !r
let () = print_int (f (fun _ -> raise Exit) succ 41)
```

## Results

OxCaml trunk aadb3804 (`5.4.0+ox`), `--enable-flambda-invariants`, amd64 Linux.

```
OCAMLRUNPARAM=b ocamlopt -Oclassic -flambda2-expert-inline-effects-in-cmm prog.ml -o p.exe
# expected: compiles, prints 41
# oxcaml:   exit 2, no executable:
Fatal error: exception Invalid_argument("Array.iter2: arrays must have the same length")
Raised at Stdlib.invalid_arg in file "stdlib.ml" (inlined), line 40, characters 20-45
Called from Stdlib__Array.iter2 in file "array.ml" (inlined), line 148, characters 4-63
Called from Cfg_selectgen.Make.emit_expr_raise in file "backend/cfg_selectgen.ml", lines 1042-1044, characters 4-98
Called from Cfg_selectgen.Make.emit_new_sub_cfg in file "backend/cfg_selectgen.ml", line 1444, characters 12-49
...
# prints 41 with: default, -Oclassic, -O3, the flag alone, -O3 with the flag, bytecode
```
