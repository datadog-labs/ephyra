# `-Oclassic -flambda2-expert-inline-effects-in-cmm`: `let f x = let y = sin x in (y, y)` fails with "Duplicate order for bindings when flushing"

Posted as https://github.com/oxcaml/oxcaml/issues/7704 on 2026-10-23. The text below is the issue as posted.

---

## Program (`prog.ml`)

```ocaml
let f x = let y = sin x in (y, y)
```

## Results

OxCaml trunk aadb3804 (`5.4.0+ox`), `--enable-flambda-invariants`, amd64 Linux.

```
ocamlopt -Oclassic -flambda2-expert-inline-effects-in-cmm -c prog.ml
# expected: compiles
# oxcaml:   exit 2, no object file:
>> Fatal error: Duplicate order for bindings when flushing: to_cmm_split_2/334 =
((expr (extcall "sin"{prog.ml:1,18-23} (load float64 x/328) float->float))
 (free_vars { x/328 }))
...
# compiles (and prints 0.841471 0.841471 with a main added) with: default, -Oclassic,
#   -O3, -flambda2-expert-inline-effects-in-cmm alone, -O3 with the flag
```
