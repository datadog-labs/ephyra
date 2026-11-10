The same eager initialization also reaches proof-carrying array accesses. There it is not a
stray panic but an unchecked out-of-bounds read, in code that proved its index in bounds.

```lean
@[inline] def bump (xs : Array UInt64) (i : Nat) : Array UInt64 :=
  if h : i < xs.size then xs.set i (xs[i] + 1) else xs

def main : IO Unit := IO.println (bump #[4] 1)
```

```sh
lean --run Set.lean
# #[4]
lean -c Set.c Set.lean && leanc -O3 -o Set Set.c && ./Set
# LEAN ASSERTION VIOLATION
# File: .../include/lean/lean.h
# Line: 1049
# i < lean_array_size(o)
```

`xs[i]` under `h : i < xs.size` is extracted as a closed term (`#[4][1]`), and the boxed
`UInt64` constant forces it at module initialization. Without `lean.h` assertions, this is a
read past the end of the array. From the generated C (nightly-2026-10-30):

```c
static uint64_t _init_l_main___closed__3(void){   // lazy, via lean_uint64_once
  ...
  v___x_88_ = lean_array_fget_borrowed(v___x_87_, v___x_86_);   // #[4], index 1
  v___x_89_ = lean_unbox_uint64(v___x_88_);
  ...
static lean_object* _init_l_main___closed__5___boxed__const__1(void){
  ...
  v___x_93_ = lean_uint64_once(&l_main___closed__4, &l_main___closed__4_once, _init_l_main___closed__4);
  v___x_94_ = lean_box_uint64(v___x_93_);
  ...
// initialize_<module>, unconditionally:
l_main___closed__5___boxed__const__1 = _init_l_main___closed__5___boxed__const__1();
```

The library hits it too. `Array.modifyM` documents "If the index is invalid, the array is
returned unmodified and `f` is not called", but with an out-of-range constant index on a
`UInt64` literal array the compiled program aborts the same way:

```lean
def f (_u : Unit) : List Nat :=
  let xs : Array UInt64 := #[12, 17, 11]
  (Array.modifyM (m := Id) xs 3 (fun (x : UInt64) => (pure (x + 1) : Id UInt64))).run.toList.map UInt64.toNat
def main : IO Unit := IO.println (f ())
```

```sh
lean --run ModifyM.lean
# [12, 17, 11]
lean -c ModifyM.c ModifyM.lean && leanc -O3 -o ModifyM ModifyM.c && ./ModifyM
# LEAN ASSERTION VIOLATION ... i < lean_array_size(o)
```

Both fail on 4.37.0-nightly-2026-10-30, master b21ac7949a, and 4.35.0, 4.33.0, 4.29.0, 4.28.0
and 4.22.0 (macOS arm64, official toolchains), and on master on Linux x86_64. The interpreter
prints the expected value on all of them.

Found by an automated differential tester ([ephyra](https://github.com/datadog-labs/ephyra)); this comment was drafted with AI assistance.
