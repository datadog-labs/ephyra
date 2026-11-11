# Bugs: elixir

The bugs the case studies found in elixir. Each folder has the report as
it was filed upstream, the reproduction, and where we had one, the fix and
a test. Status as of this export; the upstream link is authoritative.

| Bug | Status | Upstream |
|---|---|---|
| [Guard on a `<<x::bitstring>>` clause is ignored when typing the next clause](elixir-bitstring-segment-guard-refinement-deduc/) | fixed | [link](https://github.com/elixir-lang/elixir/issues/16109) |
| [False "incompatible types in binary construction" for a `::bits` pattern variable used as `::binary`](elixir-bitstring-segment-var-static-type-false-/) | fixed | [link](https://github.com/elixir-lang/elixir/issues/16106) |
| [A call argument's expected type refines the variable for later arguments of the same call](elixir-call-arg-domain-refinement-leaks-into-la/) | open | [link](https://github.com/elixir-lang/elixir/issues/16110) |
| [A clause head that cannot match is subtracted from the next clause's domain](elixir-failed-head-refinement-deducted-from-nex/) | fixed | [link](https://github.com/elixir-lang/elixir/issues/16107) |
| [Type checker crashes (MatchError in `Of.refine_body_var/5`) when a `for` generator uses a variable bound in `into:` or `reduce:`](elixir-for-option-var-used-in-generator-crashes/) | fixed | [link](https://github.com/elixir-lang/elixir/issues/16102) |
| [`Map.from_keys/2` with mixed keys drops the atom keys from the inferred type](elixir-maps-from-keys-mixed-atom-keys-dropped/) | fixed | [link](https://github.com/elixir-lang/elixir/issues/16105) |
| [`min/2` with a dynamic argument returns a non-dynamic type](elixir-min-max-dynamic-arg-result-not-gradual/) | fixed | [link](https://github.com/elixir-lang/elixir/issues/16111) |
| [False "clause will never match" for a list pattern whose tail is a cons pattern with variables (`[y \| [_]]`)](elixir-nested-cons-pattern-var-refinement-fails/) | fixed | [link](https://github.com/elixir-lang/elixir/issues/16101) |
| [False "pattern will never match" on a def head when a nested fn guard refines a head variable](elixir-nested-guard-refinement-blames-outer-pat/) | fixed | [link](https://github.com/elixir-lang/elixir/issues/16108) |
