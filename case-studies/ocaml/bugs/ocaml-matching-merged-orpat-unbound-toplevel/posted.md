# toplevel "Fatal error: n unbound at toplevel" on a redundant match case whose or-pattern binds a variable

Posted as https://github.com/ocaml/ocaml/issues/15214 on 2026-10-28. The text below is the issue as posted.

---
## Program (`m.ml`)

```ocaml
let f x = match x with (`B m | `C m), _ -> 1 | (`B n | `C n), (1 | 2) -> n
let () = print_int (f (`B 5, 1)); print_newline ()
```

The second case is redundant (warning 11), so the program should print `1`.

## Results

```
ocaml m.ml
# expected:                     1
# trunk 5a8bec5 (2026-10-28):   >> Fatal error: n unbound at toplevel
#                               Exception: Misc.Fatal_error.
# 5.4.1:      the same

ocamlc m.ml -o m.byte && ./m.byte      # 1
ocamlopt m.ml -o m.exe && ./m.exe      # 1
```
