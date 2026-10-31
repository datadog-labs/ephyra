# Bugs: binaryen

The bugs the case studies found in binaryen. Each folder has the report as
it was filed upstream, the reproduction, and where we had one, the fix and
a test. Status as of this export; the upstream link is authoritative.

| Bug | Status | Upstream |
|---|---|---|
| [`--translate-to-exnref` / `--emit-exnref` produce invalid IR when a delegate target's unreachable `try` body ends in a none-typed instruction](binaryen-translate-to-exnref-invalid-trampoline-b/) | open | [link](https://github.com/WebAssembly/binaryen/issues/9291) |
