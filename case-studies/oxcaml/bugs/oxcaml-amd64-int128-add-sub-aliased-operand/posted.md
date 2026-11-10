# Posted: https://github.com/oxcaml/oxcaml/issues/7888

Filed by alpaylan, 2026-11-05. Text as posted:

**amd64: `caml_int128_add` / `caml_int128_sub` builtins compute a wrong high word when the low half of `x` is the high half of `y`**

## Program (`prog.ml`)

```ocaml
external box : int64_u -> int64 = "%box_int64"
external sub128 : x0:int64_u -> x1:int64_u -> y0:int64_u -> y1:int64_u -> #(int64_u * int64_u) = "" "caml_int128_sub" [@@noalloc] [@@builtin]
external add128 : x0:int64_u -> x1:int64_u -> y0:int64_u -> y1:int64_u -> #(int64_u * int64_u) = "" "caml_int128_add" [@@noalloc] [@@builtin]

(* high word of (b:a) - (a:c) and of (b:a) + (a:c) *)
let[@inline never] sub_hi a b c = let #(_, hi) = sub128 ~x0:a ~x1:b ~y0:c ~y1:a in box hi
let[@inline never] add_hi a b c = let #(_, hi) = add128 ~x0:a ~x1:b ~y0:c ~y1:a in box hi

let () = Printf.printf "%Ld %Ld\n" (sub_hi #5L #100L #1L) (add_hi #5L #100L #1L)
```

`stubs.c`:

```c
void caml_int128_add(void) { __builtin_trap(); }
void caml_int128_sub(void) { __builtin_trap(); }
```

## Results

OxCaml trunk 0d3c93580d (`5.4.0+ox`), amd64 Linux, configured with `--enable-flambda-invariants`.
`(100:5) - (5:1) = (95:4)` and `(100:5) + (5:1) = (105:6)`; neither low half borrows or carries.

```
ocamlopt.opt prog.ml stubs.c -o p.exe && ./p.exe
# expected: 95 105
# oxcaml:   96 106

# -O3, -O2, -O3 -regalloc irc:   96 106
# -Oclassic, -O3 -regalloc ls:   95 105
```
