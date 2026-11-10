# Bugs: oxcaml

The bugs the case studies found in oxcaml. Each folder has the report as
it was filed upstream, the reproduction, and where we had one, the fix and
a test. Status as of this export; the upstream link is authoritative.

| Bug | Status | Upstream |
|---|---|---|
| [amd64: `Atomic.compare_and_set` on an `int Atomic.t` succeeds after an earlier failed CAS with the same expected value](oxcaml-amd64-atomic-cas-cmpxchg-rax/) | open | [link](https://github.com/oxcaml/oxcaml/issues/7889) |
| [amd64: `ocamlopt -fno-avx` fails with `Fatal error: exception Not_found` on a store to a float32 Bigarray](oxcaml-amd64-fno-avx-regalloc-not-found/) | open | [link](https://github.com/oxcaml/oxcaml/issues/7919) |
| [amd64: `caml_int128_add` / `caml_int128_sub` builtins compute a wrong high word when the low half of `x` is the high half of `y`](oxcaml-amd64-int128-add-sub-aliased-operand/) | open | [link](https://github.com/oxcaml/oxcaml/issues/7888) |
| [Bytecode: `Effect.perform` segfaults `ocamlrun` when a memprof callback grows the fiber stack](oxcaml-bytecode-perform-stack-realloc-uaf/) | open | [link](https://github.com/oxcaml/oxcaml/issues/7918) |
| [ocamlopt assertion in amd64 emit.ml for a curried function whose first parameter is an unboxed tuple of 254 or more words](oxcaml-curry-closure-large-unboxed-param/) | open | [link](https://github.com/oxcaml/oxcaml/issues/7879) |
| [x86-64 binary emitter encodes 32-bit `bswap` of r8d-r15d without REX.B, so it swaps eax-edi instead](oxcaml-internal-assembler-bswap32-missing-rex-b/) | open | [link](https://github.com/oxcaml/oxcaml/issues/7700) |
| [`ocamlopt -Oclassic` crashes on a `float#` field read in an unreachable match arm](oxcaml-oclassic-closure-conversion-block-load-k/) | open | [link](https://github.com/oxcaml/oxcaml/issues/7683) |
| [`ocamlopt -Oclassic` crashes on a division by constant zero inside a `try`](oxcaml-oclassic-div-by-const-zero-raise-ignores/) | fixed | [link](https://github.com/oxcaml/oxcaml/issues/7668) |
| [with `-flambda2-reaper -g -flambda2-expert-phantom-lets`, an unused variable in a closure breaks compilation](oxcaml-reaper-phantom-let-no-source-becomes-inv/) | open | [link](https://github.com/oxcaml/oxcaml/issues/7684) |
| [`-use-ssa`: a call followed by an unused pure operation becomes a tail call only in the SSA pipeline, validator aborts with "CFG comparison MISMATCH"](oxcaml-ssa-tail-call-accepts-dead-ops-before-re/) | open | [link](https://github.com/oxcaml/oxcaml/issues/7702) |
| [`-use-ssa`: SSA validator rejects `x <= 1` against `x < 2` for the same switch, compilation aborts with "CFG comparison MISMATCH"](oxcaml-ssa-validate-int-test-threshold-mismatch/) | open | [link](https://github.com/oxcaml/oxcaml/issues/7701) |
| [`-Oclassic -flambda2-expert-inline-effects-in-cmm`: `let f x = let y = sin x in (y, y)` fails with "Duplicate order for bindings when flushing"](oxcaml-to-cmm-double-split-must-inline-and-dupl/) | open | [link](https://github.com/oxcaml/oxcaml/issues/7704) |
| [`-Oclassic -flambda2-expert-inline-effects-in-cmm`: a call used as the argument of another call inside a `try` crashes ocamlopt with `Invalid_argument("Array.iter2 ...")`](oxcaml-to-cmm-inlined-apply-reraise-wrapper-und/) | declined | [link](https://github.com/oxcaml/oxcaml/issues/7705) |
