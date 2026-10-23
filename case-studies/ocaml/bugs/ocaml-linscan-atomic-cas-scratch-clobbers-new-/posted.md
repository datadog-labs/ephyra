# `ocamlopt -linscan` miscompiles `Atomic.compare_and_set` on amd64

Posted as https://github.com/ocaml/ocaml/issues/15189 on 2026-10-22. The text below is the issue as posted.

---

## Program (`repro/value.ml`)

```ocaml
let g = Atomic.make 0
let () =
  let v = Array.length Sys.argv in
  ignore (Atomic.compare_and_set g (Atomic.get g) v);
  print_int (Atomic.get g); print_newline ()
```

Run without arguments, `v` is 1, so the CAS succeeds and `g` must become 1.

A second form crashes (`repro/crash.ml`):

```ocaml
let n = Array.length Sys.argv
let g = Atomic.make 0
let () = ignore (Atomic.compare_and_set g (Atomic.get g) n)
```


## Results

Trunk 3c41bbf8 (2026-10-21, current head: `gh api .../compare/3c41bbf8...trunk` says "identical"),
default configure (closure), x86_64 Linux, single domain. Checked 2026-10-22 on the agent host with an
unmodified build of that commit:

```
ocamlc value.ml -o value.byte && ./value.byte
# 1
ocamlopt value.ml -o value.exe && ./value.exe
# 1
ocamlopt -linscan value.ml -o value.exe && ./value.exe
# 0

ocamlc crash.ml -o crash.byte && ./crash.byte
# (no output, exit 0)
ocamlopt crash.ml -o crash.exe && ./crash.exe
# (no output, exit 0)
ocamlopt -linscan crash.ml -o crash.exe && ./crash.exe
# Segmentation fault, exit 139
```

## Released versions are not affected

Docker `ocaml/opam:ubuntu-24.04-ocaml-5.4` (5.4.1), x86_64, checked 2026-10-22: `value.ml` prints `1`,
`crash.ml` exits 0, `repro/opaque.ml` prints `T`, all with `-linscan`. There `compare_and_set` is a C
call (`call caml_atomic_cas_field@PLT` in `-S`). The native CAS comes from PR #14980, merged
2026-10-18 (3088b2da0); `git tag --contains 3088b2da0` is empty (tags up to 5.5.1 present).
