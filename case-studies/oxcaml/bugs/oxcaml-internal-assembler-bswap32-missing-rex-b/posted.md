# x86-64 binary emitter encodes 32-bit `bswap` of r8d-r15d without REX.B, so it swaps eax-edi instead

Posted as https://github.com/oxcaml/oxcaml/issues/7700 on 2026-10-23. The text below is the issue as posted.

---

## Program (`bswap.ml`)

```ocaml
let[@inline never] f s =
  let a = String.get_int32_be s 0 in
  let b = String.get_int32_be s 1 in
  let c = String.get_int32_be s 2 in
  let d = String.get_int32_be s 3 in
  let e = String.get_int32_be s 4 in
  let g = String.get_int32_be s 5 in
  Int32.(to_int a + to_int b + to_int c + to_int d + to_int e + to_int g)
let () = print_int (f "abcdefghij"); print_newline ()
```

## Results

OxCaml trunk aadb3804 (`5.4.0+ox`), amd64 Linux.

```
ocamlc.opt bswap.ml -o b.byte && ./b.byte
# bytecode: 10055672679
ocamlopt.opt bswap.ml -o b.ext && ./b.ext
# expected: 10055672679
ocamlopt.opt -internal-assembler bswap.ml -o b.int && ./b.int
# oxcaml:   Segmentation fault, exit 139, nothing printed
# same with -O3 -internal-assembler
```

`objdump -d` of `ocamlopt.opt [-internal-assembler] -c bswap.ml`, the fifth swap:

```
# system assembler:    41 0f c8   bswap %r8d
# -internal-assembler:    0f c8   bswap %eax
```
