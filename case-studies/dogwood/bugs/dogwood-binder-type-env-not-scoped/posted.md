# Posted: https://github.com/dogwood-policy/dogwood/issues/22

Filed by alpaylan, 2026-11-02. Text as posted:

**Temporal validator types a bound variable by the last same-named binder in the condition, not the binder in scope**

## Symptom

When one temporal condition has two binders with the same name and different declared types, `dogwood validate` checks every use of that name against one of the two types, whichever binder it walked last. It accepts a use that contradicts its own binder's declared type, and it rejects some well-typed policies.

## Version

- Dogwood main 4ec5e2c01b44ff3882a231ae5d315b330b38fa25 (tag `v1.1`), still the head of the default branch on 2026-11-02, and crates.io `dogwood-cli` 1.1.0 (same output).

## Minimal input

Action schema `schema.cedarschema`:

```
namespace App {
  type In = { user: String, n: Long };
  entity Gateway;
  entity User;
  action "Login" appliesTo {
    principal: [User], resource: [Gateway],
    context: { input: In }
  };
}
```

Policy `policy.dw` (missed error):

```
permit (principal, action == App::Action::"Login", resource);

forbid (principal, action == App::Action::"Login", resource)
when temporal {
    (exists (u: Long). App::Action::"Login"::request{ input.user: u })
    && (exists (u: String). App::Action::"Login"::request{ input.user: u })
};
```

The first `u` is declared `Long` and used as `input.user`, which is a `String`.

Policy `policy-spurious.dw` (spurious error):

```
permit (principal, action == App::Action::"Login", resource);

forbid (principal, action == App::Action::"Login", resource)
when temporal {
    (exists (u: Long). App::Action::"Login"::request{ input.n: u })
    && (exists (u: String). App::Action::"Login"::request{ input.user: u })
};
```

Every use here matches its own binder's declared type.

Event schema: the default (pinned), no `--event-schema`.

Trace `trace.log` (one line):

```
@1 scope(principal: App::User::"alice", resource: App::Gateway::"gw") request_context(input: { user: "alice", n: 1 }) App::Action::"Login"::request(input: { user: "alice", n: 1 }, callerPrincipal: App::User::"alice", callerResource: App::Gateway::"gw", requestId: "r1")
```

## Commands

```
dogwood validate policy.dw --policy-schema schema.cedarschema
# spec: error: argument `input.user` of `Login` expects `string` but got `int` (exit 2)
# dogwood: OK: validation passed with no errors or warnings. (exit 0)
```

Control: `policy-renamed.dw`, the same policy with the second binder renamed to `w` (`exists (w: String). App::Action::"Login"::request{ input.user: w }`):

```
dogwood validate policy-renamed.dw --policy-schema schema.cedarschema
# spec: error (exit 2)
# dogwood: error: argument `input.user` of `Login` expects `string` but got `int`
#          FAILED: 1 error(s), 0 warning(s).   (exit 2)
```

```
dogwood validate policy-spurious.dw --policy-schema schema.cedarschema
# spec: OK, no errors (exit 0)
# dogwood: error: argument `input.n` of `Login` expects `int` but got `string`
#          FAILED: 1 error(s), 0 warning(s).   (exit 2)
```

```
dogwood replay policy.dw --policy-schema schema.cedarschema --trace trace.log
# spec: the policy should have been rejected at validation
# dogwood: @1 (time point 0): DENY  [rules: 1]
```

Decision point @1: the verdict is DENY. `policy-renamed.dw` (rejected by validation) and the same policy with both binders declared `String` also give DENY. `dogwood replay` does not refuse a policy that fails validation. The bug only changes what validation reports, not the verdict.

## Expected behaviour (spec)

`dogwood-docs/guide/04-temporal-expressions.md`, §"Binders: `exists` and `tp`", line 291:

> The annotation is **authoritative**: validation seeds the binder's declared type into the type environment and then checks every *use* of the variable against it, rather than inferring the type from the first use. A use that contradicts the declaration is a type error — `exists (x: Long). x == "s"` is rejected because the string literal is inconsistent with the declared `Long`. The annotation is only consulted at *validation* time; at *evaluation* time only the binder name matters

`dogwood-docs/guide/08-formal-specification.md`, §5.5, lines 998-1003:

> - **Types** (best-effort) — from a monomorphic environment seeded by the concrete scoped action's input fields **and every binder's declared annotation** (`exists (x: T)` and each aggregation `for (v: T)`; the declared type is **authoritative**, not reverse-inferred from a use site). Against that environment: a predicate arg must match its declared field type; a use of a bound variable must be consistent with its declared type

Reusing a binder name is legal. `04-temporal-expressions.md:558` and `08-formal-specification.md:805` both describe what a shadowing binder does ("a shadowing binder cuts inherited restrictions of its name"; "a shadowing binder removes its name from the inherited seed").

Note: §5.5 calls the type check "best-effort" and the environment "monomorphic". The spurious rejection of `policy-spurious.dw` is not a missed check, though: it reports an error on a use that matches its declared type.

## Engines

The validator does not depend on the engine. Through the public API (`Validator::validate`, then the default engine, `InMemoryTemporalEngine::new().slice_leaves()` and `partition_temporal()`, default event schema):

```
policy.dw
# dogwood: validator errors: 0
# dogwood: global: @1 Deny
# dogwood: slice_leaves: @1 Deny
# dogwood: partitioned: @1 Deny
```

```
policy-spurious.dw
# dogwood: validator errors: 1
# dogwood: global: @1 Deny
# dogwood: slice_leaves: @1 Deny
# dogwood: partitioned: @1 Deny
```

All three engines agree.

## Variants (all re-run with `dogwood validate` on trunk 4ec5e2c0 and 1.1.0, the schema above)

Each one is the body of `forbid (principal, action == App::Action::"Login", resource) when temporal { ... };`, where `L` stands for `App::Action::"Login"::request`.

Accepted with no error (missed error):
- `(exists (u: Long). L{ input.user: u }) && (exists (u: String). L{ input.user: u })` (sibling binders)
- `exists (u: Long). (L{ input.user: u } && exists (u: String). L{ input.user: u })` (nested binder)
- `exists (u: Long). (L{ input.user: u } && (count for (u: String). where (formerly within 1m L{ input.user: u })) > 1)` (aggregation `for` binder)
- `exists (u: Long). (L{ input.user: u } && count_distinct_within(u, 1m, L{ input.user: u }) > 1)` (the stdlib macro `count_distinct_within` expands to `count for (?k: String)`, `configuration/default_macros.dw:30-32`)

Rejected although well-typed (spurious error):
- `(exists (u: Long). L{ input.n: u }) && (exists (u: String). L{ input.user: u })`: "argument `input.n` of `Login` expects `int` but got `string`"
- `(exists (u: String). L{ input.user: u }) && (exists (u: Long). L{ input.n: u })`: "argument `input.user` of `Login` expects `string` but got `int`"
- `exists (u: Long). (L{ input.n: u } && exists (u: String). L{ input.user: u })`: "argument `input.n` of `Login` expects `int` but got `string`"
- The documented `count_distinct_within` idiom next to an unrelated `u: Long`: `(exists (u: Long). L{ input.n: u } && u > 3) && (exists (d: Long). (count_distinct_within(u, 1m, L{ input.user: u }) == d && d > 2))` is rejected with "argument `input.n` of `Login` expects `int` but got `string`" and "comparison requires numeric operands on both sides, got `string` and `int`". The macro expands to `count for (?k: String)`, and that hidden `String` binder replaces the user's `u: Long`. Renaming the first binder to `k` makes it validate.

Behaves as specified (different names, or the order puts the right binder last):
- `(exists (u: Long). L{ input.user: u }) && (exists (w: String). L{ input.user: w })`: rejected
- `exists (u: Long). (L{ input.user: u } && exists (w: String). L{ input.user: w })`: rejected
- `exists (u: Long). (L{ input.user: u } && (count for (v: String). where (formerly within 1m L{ input.user: v })) > 1)`: rejected
- `(exists (u: String). L{ input.user: u }) && (exists (u: Long). L{ input.user: u })`: rejected. It reports two errors, one per use, so the `String` use is also checked as `Long`.
- `exists (u: Long). L{ input.user: u }`: rejected

## Suspected code location

`dogwood-language/src/extension/temporal/validate.rs`: `check_types_for_action` (line 421) builds a single environment with `build_type_env` (line 726). That calls `collect_declared_binders` (line 751), which walks the whole condition and inserts each `exists` binder and each aggregation `for` binder into one flat `BTreeMap<String, RichType>`. A later binder of the same name overwrites an earlier one. The comment there says "the innermost (last-walked) wins, matching lexical scoping for the shapes the language admits". That does not hold for sibling binders, or for an inner binder whose scope does not cover the outer uses. `collect_declared_binder_types` (line 630, used for `sum` summand diagnostics) builds the same kind of flat map.

## Can it flip a Deny to an Allow?

Not at evaluation time. The spec says the annotation is only consulted at validation time. In the repro, all three engines give the same verdict whether or not the validator reports the error. The effects are a missed type error, and a spurious error that rejects a valid policy. A deployment that only loads policies that pass `Validator::validate()`, as the README advises, would refuse a valid policy, for example the `count_distinct_within` `forbid` above.

Filed as a GitHub issue at the Dogwood maintainers' request.

Found by an automated property-based tester (ephyra); this report was drafted with AI assistance, and every command above was re-run on trunk 4ec5e2c0 and crates.io 1.1.0.
