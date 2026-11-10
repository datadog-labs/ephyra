### Prerequisites

* [x] Check that your issue is not already filed: searched for `extract_closed`, `InferBorrow`, `borrow inference`, `borrowed join point`, `lean_is_string` and `closed term extraction` in issues and PRs.
* [x] Reduce the issue to a minimal, self-contained, reproducible test case. Avoid dependencies to Mathlib or Batteries.
* [x] Test your test case against the latest nightly release.

### Description

Compiler options must not change what a program computes. With
`set_option compiler.extract_closed false` (or `-Dcompiler.extract_closed=false`), a
string literal, or a `Nat` literal above `maxSmallNat`, that flows into a join point can
end up in a borrowed join point parameter, even though the join point body also consumes it.
The compiled program then uses the object after it has been freed. A string literal trips
`LEAN ASSERTION VIOLATION ... lean_is_string(o)`. A big `Nat` literal gives a wrong result
without any error. With closed term extraction on (the default), the same programs are
correct.

### Context

Found by an automated differential tester that compiles the same programs with and without
individual compiler options. Lean uses this option itself, in
`src/Init/System/IO.lean` (`unsafeBaseIO`) and in `bv_decide`'s
`Bitblast.lean` (`mkAuxDecl`). Neither of those reaches this bug as far as we can tell.

### Steps to Reproduce

`Nat.lean` gives a wrong value:

```lean
@[noinline] def mkN (_s : Nat) (n : Nat) : List Nat := List.replicate n 0
@[noinline] def h (n : Nat) : Nat :=
  let r := mkN 100000000000000000000000 n
  (match r with | [] => 7 | _ :: _ => 100000000000000000000000) + (100000000000000000000000 + n)
def main : IO Unit := IO.println (h 1)
```

```sh
lean -c Nat.c Nat.lean && leanc -O3 -o Nat Nat.c && ./Nat
# 200000000000000000000001
lean -Dcompiler.extract_closed=false -c Nat.c Nat.lean && leanc -O3 -o Nat Nat.c && ./Nat
# 100000000000000000000001
```

`String.lean` trips an assertion:

```lean
@[noinline] def mk (_s : String) (n : Nat) : List Nat := List.replicate n 0
@[noinline] def g (n : Nat) : Bool :=
  let r := mk "" n
  (match r with | [] => "ab" | _ :: _ => "").endsWith (("".drop 6).toString.toLower)
def main : IO Unit := IO.println (g 1)
```

```sh
lean -c String.c String.lean && leanc -O3 -o String String.c && ./String
# true
lean -Dcompiler.extract_closed=false -c String.c String.lean && leanc -O3 -o String String.c && ./String
# LEAN ASSERTION VIOLATION
# File: .../include/lean/lean.h
# Line: 769
# lean_is_string(o)
```

**Expected behavior:** both builds print the same value (`200000000000000000000001` and
`true`).

**Actual behavior:** with `compiler.extract_closed false`, `Nat.lean` prints
`100000000000000000000001` and `String.lean` fails the `lean_is_string(o)` assertion.

### Versions

```
4.37.0-nightly-2026-10-30 (f0fc2fc188)    Nat: wrong value   String: assertion
master b21ac7949a (2026-11-01)            Nat: wrong value   String: assertion
4.35.0, 4.34.1, 4.33.0, 4.30.0-rc2, 4.29.0, 4.28.0
                                          Nat: wrong value   String: assertion
4.22.0                                    Nat: wrong value   (String.lean needs String.Slice, not in 4.22)
```

These are on macOS arm64 (official toolchains via elan) and on Linux x86_64 (master built
from source, `relwithassert`). On the `relwithassert` build, `Nat.lean` fails
`LEAN ASSERTION VIOLATION ... src/runtime/object.h:178 is_mpz(o)` instead of printing the
wrong value.

### Additional Information

The bug predates the port of borrow inference to LCNF (#12413), since 4.22.0 and 4.28.0
already give the wrong `Nat` result.

### Impact

Add :+1: to issues you consider important. If others are impacted by this issue, please ask
them to add :+1: to it.

Found by an automated differential tester ([ephyra](https://github.com/datadog-labs/ephyra)); this report was drafted with AI assistance.
