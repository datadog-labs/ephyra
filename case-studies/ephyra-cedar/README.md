# ephyra-cedar

A testing engine for [Cedar](https://github.com/cedar-policy/cedar), the
authorization policy language (its Rust implementation; a Lean model is
verified and differentially tested against it in
[cedar-spec](https://github.com/cedar-policy/cedar-spec)), built on ephyra
(`../../crates/ephyra`): typed generators whose shrinkers and mutators
`ephyra check` verifies, oracles Cedar's existing tests lack, every failure
shrunk to a minimal case. It tests the stateful entity-store API, which
cedar-spec never compares with Lean, against a model, and found two bugs
there within seconds.

## Cedar

The crate depends on Cedar by path (`../../cedar/cedar-policy`,
`../../cedar/cedar-policy-core`) and tests whatever is checked out there.
Clone it at the repository root:

```
git clone https://github.com/cedar-policy/cedar && git -C cedar checkout ebe22187df775140d35661aac15b14a3a5023a3d
```

That is Cedar main of 2026-10-21 (4.14.0 in its manifests; cargo rewrites
the three cedar versions in `Cargo.lock` if they differ). It includes the
fix of the first bug below (#2628), not the second: until
[PR #2633](https://github.com/cedar-policy/cedar/pull/2633) lands,
`entity_updates_match_model`, `readding_an_identical_entity_is_accepted`
and `added_parents_are_kept` fail on Cedar main. To test past it, apply the
fix:

```
git -C cedar apply ../case-studies/bugs/cedar/cedar-add-entities-closure-duplicate/fix.patch
```

With it, all four tests pass (`entity_updates_match_model` at 100,000
traces).

## What exists

- [src/hierarchy.rs](src/hierarchy.rs): traces of
  `Entities::add_entities` / `upsert_entities` / `remove_entities` on
  entities with parents only (ids `E::"0"` to `E::"7"`), the generator
  `traces` (shrinks calls, entries and parents; metrics: calls, failing
  calls, upserts naming an entity twice, deepest closure, stored at end),
  and a model (`step`) that keeps declared parents only, as the
  documentation describes each call, and recomputes reachability.
- [tests/entities.rs](tests/entities.rs): the property
  `entity_updates_match_model` (after every call, same success and same
  ancestors for every entity), and the bugs as explicit cases on users and
  groups, each with an authorization decision. On main before either fix it
  fails within about 200 traces (the duplicate bug); with only the
  `add_entities` fix, within 90,000 to 450,000 (the upsert bug, every seed
  tried); each shrinks to two or three calls.
- `traces` metrics (10,000 values): calls 1 to 16 uniform; deepest closure
  median 2, p90 4, max 7; failing calls median 2; 35% of traces have an
  upsert naming an entity twice.
- [examples/ext_probe.rs](examples/ext_probe.rs): evaluates extension
  constructors on given strings in Rust Cedar (triage). Rust and Lean
  extension parsers agree on its hand-picked edge cases (year 0000, invalid
  month days, hour 24, trailing junk, i64 bounds of decimal and duration,
  `::` with seven groups); the Lean side was run with `lake env lean`.

## Findings

Folders under [../bugs/cedar](../bugs/cedar) have the issue as filed and the
fix with tests (`fix.patch`, a `git format-patch` commit with a CHANGELOG
entry and unit tests that fail without the fix).

- [cedar-policy/cedar#2606](https://github.com/cedar-policy/cedar/issues/2606),
  fixed by #2628:
  [cedar-upsert-repeated-entity-stale-ancestor](../bugs/cedar/cedar-upsert-repeated-entity-stale-ancestor/).
  A regression in 4.11.0 (#2317). An upsert batch that names an entity twice
  (documented: the latest version wins) can leave a stale ancestor:
  replacing the batch's own first version strips only that version's parents
  from its descendants, so a later entity of the batch misses a descendant,
  and `repair_tc` only adds edges. Revoked group membership survives:
  `permit(principal in Group::"root", ...)` still allows.
- [cedar-policy/cedar#2629](https://github.com/cedar-policy/cedar/issues/2629),
  fix in [PR #2633](https://github.com/cedar-policy/cedar/pull/2633):
  [cedar-add-entities-closure-duplicate](../bugs/cedar/cedar-add-entities-closure-duplicate/).
  Since 4.4.0, `add_entities` decides "identical" with `Entity::deep_eq`,
  which compares ancestor sets: the new entity's parents against the stored
  entity's closure. An identical entity with a grandparent is refused as a
  duplicate, and an entity whose parents equal the stored closure is
  accepted as identical and dropped, losing a declared parent (a later
  upsert then flips an authorization decision). The fix compares declared
  data. `fix-on-top-of-upsert-fix.patch` is the same commit rebased onto our
  fix of the first bug.

Checked, not a bug: entity data whose action entity declares exactly its
schema parents is refused ("does not match its schema declaration") when the
data omits the action's parent actions and groups nest. Action conformance
is checked after the closure is computed within the data, so the stored
action would have had the wrong closure; the error is safe, only its message
misleads. Cedar's test `issue_285` pins this behaviour under
`TCComputation::AssumeAlreadyComputed`.

## Facts about Cedar's testing that shape the design

From reading cedar-spec:

- Lean only ever receives protobuf of Rust's already-built ASTs, entities
  (with Rust's closure as their ancestors), requests and resolved schemas.
  So these are Rust-only and never compared with Lean: the policy text
  parser, EST JSON, schema text/JSON parsing and name resolution, entity and
  context JSON parsing, transitive closure and cycle detection (generated
  hierarchies are always DAGs), action entities from the schema, and the
  protobuf conversions. Nothing calls `upsert_entities`.
- Comparisons are coarse: evaluation compares value-or-error, authorization
  the decision plus determining and erroring policy ids, validation
  pass/fail in strict mode only.
- Inputs are small: ABAC targets use depth 3, width 3, one policy (or one
  template and one link); multi-policy sets only from RBAC; one namespace;
  no action attributes.
- Generators produce extension strings in canonical form, then mutate bytes
  (Latin-1 only); extended `has` takes a schema attribute first and random
  identifiers after it, so deep chains rarely follow real record paths.
- No tests in cedar-spec for PolicySet add/remove/link/unlink/merge, the
  FFI/wasm JSON entry points, cedar-policy-cli, the language server, or the
  old partial evaluator.
- Fresh code (high bug odds): extended `has` as one AST node (#2560, across
  evaluator, typechecker, TPE, level validation, symcc), fewer parentheses
  in the printer (#2520), specialized `from_normalized_str` (#2564), TPE
  schema-based reductions (#2503), and the closure algorithm rewrite (#2317,
  the regression above).

Not yet: `PolicySet` as a stateful API against a model; multi-path
consistency (the same policy as text, EST, PST and protobuf authorizes the
same; printer and formatter re-parse to the same policy; the `ffi` JSON
entry points agree with the native API); a Lean differential through
cedar-spec's `cedar-lean-ffi` with generators aimed at the gaps above.

## Verifying

`cargo test`; `cargo clippy --all-targets -- -D warnings`; rustfmt
(`rustfmt.toml` is the stable subset of ephyra's). Generators:
`cargo build -p ephyra-cli` at the repository root, then
`LLVM_PROFILE_FILE="$PWD/target/profraw/%m_%p.profraw" ../../target/debug/ephyra check`
here (it instruments by default; without the variable it leaves
`default_*.profraw` files in the crate directory).
