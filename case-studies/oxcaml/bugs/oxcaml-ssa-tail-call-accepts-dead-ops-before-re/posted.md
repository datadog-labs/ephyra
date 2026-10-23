# `-use-ssa`: a call followed by an unused pure operation becomes a tail call only in the SSA pipeline, validator aborts with "CFG comparison MISMATCH"

Posted as https://github.com/oxcaml/oxcaml/issues/7702 on 2026-10-23. The text below is the issue as posted.

---

## Program (`prog.ml`)

```ocaml
let[@inline never] g x = x + 1

let f x =
  let r = g x in
  let _ = Int64.to_float 5L in
  r

let () = print_int (f 41)
```

Flambda 2 keeps the unused conversion in Cmm (`-dcmm`):

```
(let
  (r/387 (app{prog.ml:4,10-13} G:"camlProg__g_0_2_code" x/386 int)
   param/389 (int->float 5))
  r/387)
```

## Results

OxCaml trunk aadb3804 (`5.4.0+ox`), amd64 Linux.

```
ocamlopt -use-ssa prog.ml -o p.exe
# expected: compiles; ./p.exe prints 42
# oxcaml:   no executable, exit 2:
*** CFG comparison MISMATCH for camlProg__f_1_3_code:
Terminator mismatch at old=107(id:5) new=111(id:2): call "camlProg__g_0_2_code"
           goto 109 vs tailcall "camlProg__g_0_2_code"
Result register count mismatch at old=107(id:5) new=111(id:2): old has 1 res, new has 0 res
...
>> Fatal error: CFG comparison MISMATCH for camlProg__f_1_3_code
# same with -use-ssa -O3, -use-ssa -Oclassic, and -experimental-optimizations alone
# compiles and prints 42 with -use-ssa -no-ssa-validate, and with default flags
```
