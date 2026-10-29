# flambda `ocamlopt` stops with "Bad definition for float array member" on a GADT match with a dead float-array branch

Posted as https://github.com/ocaml/ocaml/issues/15212 on 2026-10-28. The text below is the issue as posted.

---
## Program (`gadt_float.ml`)

```ocaml
type _ t = Int : int t | Float : float t
let () =
  let f (type a) (t : a t) (x : a) =
    match t with Float -> ignore [| x |] | Int -> ()
  in
  f Int 0;
  print_endline "ok"
```

Well-typed; `f Int 0` never reaches the `Float` branch, so the program should print `ok`.

## Results

```
ocamlc gadt_float.ml -o bc.exe && ./bc.exe
# bytecode (expected): ok
ocamlopt gadt_float.ml -o nat.exe
# flambda trunk fec7cc76d5 (2026-10-27), exit 2, no executable:
>> Fatal error: Bad definition for float array member x/3: 0
Fatal error: exception Misc.Fatal_error
# flambda 5.4.1: the same fatal error
```
