# amd64 `Atomic.exchange` stores a wrong value (trunk regression)

Posted as https://github.com/ocaml/ocaml/issues/15205 on 2026-10-27. The text below is the issue as posted.

---
## Program (`poc.ml`)

```ocaml
let a = Atomic.make 0
let () =
  let n = Sys.opaque_identity 2 in
  let x = Sys.opaque_identity 1 in
  let old = Atomic.exchange a x in
  for _ = x downto n lor 0 do
    if sin 1. = 0. then ignore (Sys.opaque_identity (if n = 0 then old else 0))
  done;
  print_int (Atomic.get a)
```

The loop body never runs (`1 downto 2`), so the program should print `1`.

## Results

```
ocamlc poc.ml -o poc.byte && ocamlrun ./poc.byte
# bytecode (expected):            1
ocamlopt poc.ml -o poc.exe && ./poc.exe
# trunk 2b789c3211 (2026-10-27):  66871267163816, 67222252813992, 68267097980584 on three runs
# 5.4.1:                          1
```

- `ocamlopt -version` on trunk: `5.7.0+dev0-2026-10-06`. Default configure (closure middle end),
  default register allocator, no flags.
- `repro/repro.sh <bin-dir>` (runs `poc-loop.ml`): exit 1 on trunk, 0 on 5.4.1 and on the fixed trunk.

`-dlinear` on trunk, new-value argument and result in different registers:

```
old/56[%rsi] I/37[%rdi] := atomic_exchange [V/35[%rbx]] old/57[%rdx] {poc.ml:5,12-31}
```

and the emitted instruction is `xchg %rsi, (%rbx)`, so the value loaded into `%rdx` is never stored.
With the fix: `... := atomic_exchange [V/35[%rbx]] old/56[%rsi]`.

## What it needs (each checked on trunk)

It depends on register allocation spilling `old`, so small changes hide it:
- a `[@@noalloc]` float C call in the loop: `sin`, `exp`, `log`, `atan2`, `Float.fma`, `Float.rem`,
  `Float.round` and `Float.cbrt` all reproduce.
- a computed loop bound: `n lor 0` and `n land 3` reproduce, plain `n` does not;
- `old` used in the loop through `if n = 0 then old else 0`; `Some old`, `[old]`, or moving `n = 0`
  into the outer condition do not reproduce.

## Where it comes from

- 5.4.1 is not affected: `asmcomp/amd64/selection.ml` has no `Iatomic_exchange` there (0 matches;
  2 on trunk). Native atomic exchange came with PR #14980 ("Generate native atomic exchange and
  compare-and-set across all architectures", merged 2026-10-18). Before it, `Atomic.exchange` was a C call.
  Not bisected beyond this.
- Cause (agent analysis, confirmed by the fix and the `-dlinear` above): `selection.ml` ties `res.(0)`
  to `arg.(1)` for `Iatomic_exchange` (and `Iatomic_fetch_add`, from PR #14575), because `xchg`/`xadd`
  overwrite their register operand. When that pseudo-register is spilled, `Reloadgen.reload_operation`
  reloads the argument and the result into two different registers.
- `Atomic.fetch_and_add` in the loop program's shape prints the right value (1) on trunk. The fix covers
  it as well (same tie), but no failing `fetch_and_add` program was found.

Found by Ephyra, a new PBT library I'm working on, triaged and drafted by an AI agent; I've edited and validated the report after the fact.
