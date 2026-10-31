# Bugs: ocaml

The bugs the case studies found in ocaml. Each folder has the report as
it was filed upstream, the reproduction, and where we had one, the fix and
a test. Status as of this export; the upstream link is authoritative.

| Bug | Status | Upstream |
|---|---|---|
| [amd64 `Atomic.exchange` stores a wrong value (5.6 & trunk regression)](ocaml-amd64-atomic-exchange-spill/) | open | [link](https://github.com/ocaml/ocaml/issues/15205) |
| [flambda `ocamlopt` stops with "Bad definition for float array member" on a GADT match with a dead float-array branch](ocaml-flambda-gadt-dead-float-array/) | fixed | [link](https://github.com/ocaml/ocaml/issues/15212) |
| [non-exhaustive integer match crashes instead of raising Match_failure](ocaml-int-switch-missing-upper-fail-interval/) | fixed | [link](https://github.com/ocaml/ocaml/issues/15185) |
| [`ocamlopt -linscan` miscompiles `Atomic.compare_and_set` on amd64](ocaml-linscan-atomic-cas-scratch-clobbers-new-/) | open | [link](https://github.com/ocaml/ocaml/issues/15189) |
| [toplevel "Fatal error: n unbound at toplevel" on a redundant match case whose or-pattern binds a variable](ocaml-matching-merged-orpat-unbound-toplevel/) | open | [link](https://github.com/ocaml/ocaml/issues/15214) |
