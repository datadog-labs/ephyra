# `-use-ssa`: SSA validator rejects `x <= 1` against `x < 2` for the same switch, compilation aborts with "CFG comparison MISMATCH"

Posted as https://github.com/oxcaml/oxcaml/issues/7701 on 2026-10-23. The text below is the issue as posted.

---

## Program (`prog.ml`)

```ocaml
type t = A | B | C
let f v = let _ = try (match v with A -> 0 | B -> 1 | C -> truncate (sqrt 2.0)) with Exit -> 0 in print_endline "ok"
let () = f A; f B; f C
```

## Results

OxCaml trunk aadb3804 (`5.4.0+ox`), amd64 Linux.

```
ocamlopt -use-ssa prog.ml -o p.exe
# expected: compiles; ./p.exe prints ok ok ok
# oxcaml:   no executable, exit 2:
*** CFG comparison MISMATCH for camlProg__f_0_1_code:
Label mapping conflict: old=129 already maps to new=143, not new=142
Label mapping conflict: new=142 already maps to old=129, not old=120
Terminator mismatch at old=101(id:35) new=138(id:7): if < u 1 goto 129if = u 1 goto 129if > u 1 goto 120 vs if < u 2 goto 143if = u 2 goto 142if > u 2 goto 142
...
>> Fatal error: CFG comparison MISMATCH for camlProg__f_0_1_code
# same with -use-ssa -O3 and with -experimental-optimizations alone
# compiles and prints ok ok ok with -use-ssa -no-ssa-validate, and with default flags
```
