# Posted: https://github.com/dogwood-policy/dogwood/issues/23

Filed by alpaylan, 2026-11-02. Text as posted:

**`dogwood validate` drops the input/output fields of an action whose context names a top-level common type from inside a namespace**

Symptom: inside `namespace App`, if an action's context is a bare name for a common type declared at top level (`context: Ctx`), the derived event schema has no `input.*` / `output.*` fields. `dogwood validate` then rejects any temporal predicate on those fields.

Version: Dogwood main 4ec5e2c01b44ff3882a231ae5d315b330b38fa25 (tag `v1.1`), still the head of the default branch on 2026-11-02, and crates.io `dogwood-cli` 1.1.0 (same output).

## Repro files

`policy.dw`:

```
permit (principal, action, resource)
when temporal { formerly within 1h App::Action::"Read"::response{ output.r: 1 } };
```

`bug.cedarschema` (`Ctx` declared at top level):

```
type Ctx = { input: { n: Long }, output?: { r: Long } };
namespace App {
  entity User;
  entity Doc;
  action "Read" appliesTo { principal: [User], resource: [Doc], context: Ctx };
}
```

`ok.cedarschema` (the same schema with `Ctx` moved inside `App`):

```
namespace App {
  type Ctx = { input: { n: Long }, output?: { r: Long } };
  entity User;
  entity Doc;
  action "Read" appliesTo { principal: [User], resource: [Doc], context: Ctx };
}
```

Event schema: the default (pinned). No `--event-schema` flag is needed.

`trace.log` (used only to show that replay is not affected):

```
@1 scope(principal: App::User::"alice", resource: App::Doc::"d1") request_context(input: { n: 0 }) App::Action::"Read"::request(input: { n: 0 }, callerPrincipal: App::User::"alice", callerResource: App::Doc::"d1", requestId: "q1")
@2 App::Action::"Read"::response(input: { n: 0 }, output: { r: 1 }, callerPrincipal: App::User::"alice", callerResource: App::Doc::"d1", requestId: "q1")
@3 scope(principal: App::User::"alice", resource: App::Doc::"d1") request_context(input: { n: 0 }) App::Action::"Read"::request(input: { n: 0 }, callerPrincipal: App::User::"alice", callerResource: App::Doc::"d1", requestId: "q2")
```

## Commands, observed and expected

```
dogwood validate policy.dw --policy-schema ok.cedarschema
# expected: OK: validation passed with no errors or warnings.
# dogwood:  OK: validation passed with no errors or warnings.   (exit 0)
```

```
dogwood validate policy.dw --policy-schema bug.cedarschema
# expected: OK: validation passed with no errors or warnings.
# dogwood:  error: predicate `App::Action::"Read"::response` mentions field `output.r`, which is not declared on that event (declared fields: callerPrincipal, callerResource, requestId, sessionId)
#           FAILED: 1 error(s), 0 warning(s).   (exit 2)
```

Cedar resolves `Ctx` to the top-level type. Using the same `bug.cedarschema`:

```
cedar validate --schema bug.cedarschema --policies c.cedar   # c.cedar: permit (principal, action, resource) when { context.input.n == 0 };
# cedar-policy-cli 4.11.0:  ☞ policy set validation passed
#                           ╰─▶ no errors or warnings
# Dogwood: `dogwood validate` on the same Cedar policy (as .dw): OK: validation passed with no errors or warnings.
```

So Dogwood's own Cedar validation sees `context.input.n` through `Ctx`. Only the derived event schema misses it.

Replay verdicts per decision point (default engine, CLI). They are the same under both schemas, because replay does not validate:

```
dogwood replay policy.dw --policy-schema bug.cedarschema --trace trace.log
# expected: @1 (time point 0): DENY
#           @3 (time point 1): ALLOW  [rules: 0]
# dogwood:  @1 (time point 0): DENY
#           @3 (time point 1): ALLOW  [rules: 0]
```

The wrong result is the validation verdict on `bug.cedarschema`: an error where none is expected.

## Spec text

`dogwood-docs/guide/02-policy-language.md` line 94:

> **`type` declarations** are Cedar *common types* — reusable record types (`LoginInput`, `ReadOutput`, …) that the actions reference as their `input` / `output` records. Common types may be cross-namespace (`Shared::Addr`) or chained (`type Outer = Inner;`); Dogwood resolves both.

`dogwood-docs/guide/03-event-schema.md`, section "Spread selectors: `...inputs(A)` and `...outputs(A)`":

> `...inputs(A)` mints a group named **`input`** whose members are the fields of the action's `context.input` record.

> `...outputs(A)` mints a group named **`output`** from `context.output`.

Cedar name resolution, which Dogwood relies on for these schemas: a bare name used inside a namespace resolves to the name in that namespace if it is declared there, otherwise to the name in the empty namespace. cedar 4.11.0 accepts `bug.cedarschema` and resolves `context.input.n` through `Ctx`, as shown above. `ok.cedarschema` and `bug.cedarschema` describe the same context for `App::Action::"Read"`.

## Engines

The failure is in `Validator::validate`, a static check that runs before any temporal engine. The default engine, `InMemoryTemporalEngine::new().slice_leaves()` and `partition_temporal()` give the same verdicts under both schemas (@1 Deny, @3 Allow).

## Variants

These fail (same "not declared on that event" error, on 4ec5e2c0 and 1.1.0):
- `type Ctx = { input: { n: Long } };` at top level, with `context: Ctx` inside `namespace App`, and the predicate `App::Action::"Read"::request{ input.n: 0 }`.
- An inline context `context: { input: In }` inside `App`, with `type In = { n: Long };` at top level.
- `namespace Shared { type Ctx = { input: In }; }` with `type In` at top level and `context: Shared::Ctx` (a chained cross-namespace reference that ends at a top-level type).

These pass:
- `Ctx` declared inside `App` (`ok.cedarschema`).
- Everything at top level (no namespace): `type Ctx` plus `action "Read" ... context: Ctx`, with the predicate `Action::"Read"::request{ input.n: 0 }`.
- A plain Cedar condition `when { context.input.n == 0 }` with `bug.cedarschema`.

Not testable: declaring `In` both at top level and in `App`. Cedar rejects that schema ("definition of `App::In` illegally shadows the existing definition of `In`").

## Suspected code location

`dogwood-language/src/event_schema/derive.rs`, function `resolve_to_record` (line 634; the lines below are 689-694). A bare reference is resolved only in the action's home namespace:

```
// Split a qualified reference into (namespace, basename); a bare
// reference resolves in the action's home namespace.
let (target_ns, basename) = match full.rsplit_once("::") {
    Some((ns, base)) => (qualify(ns)?, base.to_string()),
    None => (home_ns.clone(), full.clone()),
};
```

If the name is not found there, the function treats it as "not a record". `context_subrecord` (line 491) then yields no fields. Making the bare case fall back to the empty namespace, as Cedar does, makes `policy.dw` validate OK on `bug.cedarschema` and on the three failing variants above, on a local build.

## Can it flip a Deny to an Allow?

No. The bug makes validation reject a valid policy. In the cases tried, no check is skipped: a mention of any input/output field is rejected outright. Replay verdicts are unchanged.

Filed as a GitHub issue at the Dogwood maintainers' request.

Found by an automated property-based tester (ephyra); this report was drafted with AI assistance, and every command above was re-run on trunk 4ec5e2c0 and crates.io 1.1.0.
