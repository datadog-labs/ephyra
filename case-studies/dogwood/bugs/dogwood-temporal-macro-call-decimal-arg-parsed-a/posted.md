# Posted: https://github.com/dogwood-policy/dogwood/issues/33

Filed by alpaylan, 2026-11-02. Text as posted:

**A `decimal("…")` literal passed as an argument to a `def temporal` macro is rejected as "unknown macro `decimal`"**

## Symptom

When a `def temporal` macro is called with a decimal literal as an argument, such as `paid(decimal("1.50"))`, the whole policy file fails to load with "unknown macro `decimal` (no declared `def temporal`)". The same policy with the macro body inlined loads and evaluates. The same macro called with a non-literal argument (`paid(context.input.amt)`) also loads and evaluates.

## Version

- Dogwood commit: 4ec5e2c01b44ff3882a231ae5d315b330b38fa25 ("Sync language source (2026-10-14) (#20)"). It was still the head of the upstream default branch on 2026-11-02. The crates.io `dogwood-cli` 1.1.0 release and a build of tag v1.0 give the same error.
- dogwood-language 1.1.0 (`dogwood --version`: `dogwood 1.1.0`).
- Built with `cargo build --release -p dogwood-cli` (a debug build gives the same output).

## Minimal input

Action schema `schema.cedarschema`:

```
namespace App {
  type In = { user: String, amt: decimal };
  entity Gateway;
  entity User;
  action "Pay" appliesTo {
    principal: [User], resource: [Gateway],
    context: { input: In }
  };
}
```

Policy `policy.dw`:

```
def temporal paid(?a) { App::Action::"Pay"::request{ input.amt: ?a } };

permit (principal, action == App::Action::"Pay", resource);

forbid (principal, action == App::Action::"Pay", resource)
when temporal {
    formerly within 5s paid(decimal("1.50"))
};
```

Policy `policy-inlined.dw` (the call `paid(decimal("1.50"))` replaced by the macro body with `?a` substituted):

```
permit (principal, action == App::Action::"Pay", resource);

forbid (principal, action == App::Action::"Pay", resource)
when temporal {
    formerly within 5s App::Action::"Pay"::request{ input.amt: decimal("1.50") }
};
```

Policy `policy-control.dw` is `policy.dw` with the call changed to `paid(context.input.amt)`, so its last block reads `formerly within 5s paid(context.input.amt)`.

Event schema: the default (pinned), no `--event-schema`.

Trace `trace.log` (one line):

```
@1 scope(principal: App::User::"alice", resource: App::Gateway::"gw") request_context(input: { user: "alice", amt: 1.5 }) App::Action::"Pay"::request(input: { user: "alice", amt: 1.5 }, callerPrincipal: App::User::"alice", callerResource: App::Gateway::"gw", requestId: "r1")
```

## Commands

```
dogwood replay policy.dw --policy-schema schema.cedarschema --trace trace.log
# spec: @1 (time point 0): DENY  [rules: 1]   (same as the inlined policy)
# dogwood: exit 2, no verdicts:
#   × unknown macro `decimal` (no declared `def temporal`)
#    ╭─[1:25]
#  1 │ def temporal paid(?a) { App::Action::"Pay"::request{ input.amt: ?a } };
#    ·                         ───────┬───────
#    ·                                ╰── unknown macro `decimal` (no declared `def temporal`)
#  2 │ 
#    ╰────
```

```
dogwood replay policy-inlined.dw --policy-schema schema.cedarschema --trace trace.log
# spec: @1 (time point 0): DENY  [rules: 1]
# dogwood: @1 (time point 0): DENY  [rules: 1]   (exit 0)
```

```
dogwood replay policy-control.dw --policy-schema schema.cedarschema --trace trace.log
# spec: @1 (time point 0): DENY  [rules: 1]
# dogwood: @1 (time point 0): DENY  [rules: 1]   (exit 0)
```

`dogwood validate policy.dw --policy-schema schema.cedarschema` fails with the same error (exit 2). `policy-inlined.dw` validates: "OK: validation passed with no errors or warnings."

## Observed and expected verdicts

Decision point @1 (one Pay request with `input.amt` = 1.5):

- Observed: none. The policy file is rejected before evaluation (exit 2).
- Expected: DENY. The forbid holds: a Pay request with `input.amt` equal to `decimal("1.50")` occurred within 5s. The inlined policy and the control policy both give DENY.

The error label points at 15 characters of the macro definition on line 1 (`App::Action::"P`), not at the call `decimal("1.50")` (also 15 characters) on line 7. The span looks like it is taken relative to a different source text and applied to the whole file.

## Spec text

`dogwood-docs/guide/09-calling-macros.md`, section "What is checked at the call site":

> - **Argument shape** — each argument must match how its parameter is used in the body: a whole-condition parameter takes a temporal condition, a window parameter takes a bare interval literal, a term parameter takes an expression, and a binder-position parameter takes a bare identifier.
>
> Calling a name that is neither a declared macro nor a Cedar built-in is rejected as an unknown call.

`?a` is used as a predicate field value, so it is a term parameter and takes an expression.

`dogwood-docs/guide/02-policy-language.md`, section "Extension functions: `decimal`, `datetime`, `duration`, `ip`":

> A bare name immediately followed by `(args)` is an extension-function call. Dogwood recognizes four built-in constructors, each taking exactly one string argument, used to build the non-primitive literal types:

The table that follows in that section lists `decimal("…")` with type `decimal` and meaning "fixed-point decimal literal". So `decimal("1.50")` is a built-in, not a call to an undeclared macro.

The temporal grammar (`dogwood-language/src/extension/temporal/grammar.pest`) has this comment on `term`: "`entity` and `decimal_lit` precede `agg_expr` because `agg_expr` includes `call = ident "(" … ")"`, which would otherwise mis-grab `decimal("1.5")` as a call named `decimal`". That mis-grab is what happens in a macro call argument.

## Engines

The error is raised while the policy file is parsed and macros are expanded (CLI load, before evaluation), before any temporal engine is chosen. So the default, slice_leaves and partitioned engines cannot be built from this policy at all. The small driver over the public API below prints, for each of the three engines:

```
cargo run --release -- policy.dw schema.cedarschema trace.log
# global: lowering failed: unknown macro `decimal` (no declared `def temporal`)
# (the same line for slice and partition)
```

<details><summary>Engines driver (Rust, public API of dogwood-language 1.1.0)</summary>

`Cargo.toml`:

```toml
[package]
name = "eng"
version = "0.1.0"
edition = "2024"

[dependencies]
dogwood-language = "=1.1.0"
```

`src/main.rs`:

```rust
// usage: eng <policy.dw> <schema.cedarschema> <trace.log> [providers.json] [event-schema]
use dogwood_language::*;
use std::time::Instant;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let src = std::fs::read_to_string(&a[1]).unwrap();
    let schema = std::fs::read_to_string(&a[2]).unwrap();
    let log = std::fs::read_to_string(&a[3]).unwrap();
    let prov = a.get(4).filter(|s| !s.is_empty()).map(|p| std::fs::read_to_string(p).unwrap());
    let evs = a.get(5).map(|p| std::fs::read_to_string(p).unwrap());
    let engines = std::env::var("ENGINES").unwrap_or("global,slice,partition".into());
    for which in engines.split(',') {
        let mut b = ServiceSchema::builder();
        if let Some(p) = &prov { b = b.providers(ProviderDeclarations::from_json(p).unwrap()); }
        if let Some(e) = &evs { b = b.event_schema_str(e); }
        let service = b.build().unwrap();
        let ps = PolicySchema::from_cedarschema_str(&schema).unwrap();
        let lowered = match LoweredPolicySet::from_str(&src, &service, &ps) {
            Ok(l) => l,
            Err(e) => { println!("{which}: lowering failed: {e}"); continue; }
        };
        let mut auth = match which {
            "global" => Authorizer::new(lowered),
            "slice" => Authorizer::builder(lowered).temporal_engine(InMemoryTemporalEngine::new().slice_leaves()).build().unwrap(),
            "partition" => Authorizer::builder(lowered).partition_temporal().build().unwrap(),
            _ => panic!(),
        };
        let events = parse_trace(&log).unwrap();
        let t = Instant::now();
        let mut out = String::new();
        for e in &events {
            if let Some(r) = auth.is_authorized(e) {
                out.push(match r.decision() { Decision::Allow => 'A', Decision::Deny => 'D' });
            }
        }
        println!("{which:9} {:>10.3?} {out}", t.elapsed());
    }
}
```

Run as `cargo run --release -- policy.dw schema.cedarschema trace.log [providers.json]`. It prints one line per engine: the name, the time spent in `is_authorized`, and one letter per decision (A = Allow, D = Deny).

</details>

## Variants

Rejected with "unknown macro `decimal`" (`dogwood replay`, trunk), all with the same schema and trace:

- `formerly within 5s paid(decimal("1.50"))` (the case above)
- `formerly within 5s paid((decimal("1.50")))` (argument in parentheses)
- `paid(decimal("1.50"))` with no `formerly` around it
- two-parameter macro with a window: `def temporal paidw(?w, ?a) { formerly within ?w App::Action::"Pay"::request{ input.amt: ?a } };` called as `paidw(5s, decimal("1.50"))`
- decimal as the second argument: `paidu("alice", decimal("1.50"))` and `paidu(context.input.user, decimal("1.50"))` with `def temporal paidu(?u, ?a) { App::Action::"Pay"::request{ input.user: ?u, input.amt: ?a } };`

Accepted, DENY at @1:

- `formerly within 5s paid(context.input.amt)` (`policy-control.dw`)
- `formerly within 5s App::Action::"Pay"::request{ input.amt: decimal("1.50") }` (inlined, `policy-inlined.dw`)
- `formerly within 5s (App::Action::"Pay"::request{} && decimal("1.50") == context.input.amt)`
- `formerly within 5s (App::Action::"Pay"::request{} && context.input.amt == decimal("1.50"))`

So `decimal("…")` is accepted everywhere tried except as a temporal macro call argument.

In the temporal grammar only `decimal(...)` has its own literal rule (`decimal_lit`). `datetime(...)`, `duration(...)` and `ip(...)` are also rejected as macro arguments ("unknown macro `ip` (no declared `def temporal`)", and likewise for `datetime` and `duration`), but unlike `decimal` they are rejected inline in temporal conditions as well: `formerly within 5s (App::Action::"Pay"::request{} && context.input.a == ip("1.2.3.4"))` gives "unknown macro `ip`", and `App::Action::"Pay"::request{ input.a: ip("1.2.3.4") }` gives "an aggregate (`sum`/`count`) may appear only as the immediate operand of a comparison ..." (with `a: ipaddr` added to `In`). That is a separate, broader gap and not part of this report.

## Suspected code location

- `dogwood-language/src/extension/temporal/grammar.pest`: `call_arg = { interval_lit | condition | term }` tries `condition` before `term`. Inside `condition`, `atom` tries `call` (`call = { ident ~ "(" ~ call_arg_list? ~ ")" }`) before `comparison`, and `call` matches `decimal("1.50")` as a call named `decimal`. The `term` rule, which puts `decimal_lit` first, is never reached.
- `dogwood-language/src/macros/mod.rs`: `expand_temporal_call_args` expands the argument as a temporal condition, and `expand_temporal_call_to_condition` fails the registry lookup with "unknown macro `{}` (no declared `def temporal`)".

Adding a `!decimal_lit` lookahead to `call` in grammar.pest (`call = { !decimal_lit ~ ident ~ "(" ~ call_arg_list? ~ ")" }`) makes `policy.dw` replay to `@1 (time point 0): DENY  [rules: 1]`. `decimal` is a reserved macro name (`RESERVED_MACRO_NAMES`), so the lookahead cannot hide a user macro.

## Deny to Allow

Not observed. The policy file is rejected as a whole with an error, so no decision is produced from it. A caller that falls back to an older or smaller policy set when loading fails could lose the forbid, but that depends on the caller, not on Dogwood.

---

Filed as a GitHub issue at the Dogwood maintainers' request.

Found by an automated property-based tester (ephyra); this report was drafted with AI assistance, and every command above was re-run on trunk 4ec5e2c0 and crates.io 1.1.0.
