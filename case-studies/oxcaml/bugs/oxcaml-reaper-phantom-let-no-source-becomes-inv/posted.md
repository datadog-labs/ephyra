# with `-flambda2-reaper -g -flambda2-expert-phantom-lets`, an unused variable in a closure breaks compilation

Posted as https://github.com/oxcaml/oxcaml/issues/7684 on 2026-10-22. The text below is the issue as posted.

---


## Program (`repro/prog.ml`)

```ocaml
let f a = fun y -> let _z = a + 1 in y + 1
let () = print_int (f 7 3); print_newline ()
```

## Results

OxCaml trunk aadb3804 (`5.4.0+ox`), amd64 Linux.

```
ocamlopt -flambda2-reaper -g -flambda2-expert-phantom-lets prog.ml -o p.exe && ./p.exe
# expected: 4
# oxcaml:   compiles; at run time prints to stderr and aborts (exit 134, SIGABRT):
[ocaml] [flambda2] Invalid code:
Dead variable in bound pattern: a/94N

This might have arisen from a wrong use of [%identity],
for example confusing arrays and records, or non-mixed and
mixed blocks.
Consider using [Obj.magic], [Obj.repr] and/or [Obj.obj].

ocamlopt prog.ml -o p.exe && ./p.exe
# oxcaml:   4, exit 0
```
