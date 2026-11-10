# Posted: https://github.com/dogwood-policy/dogwood/issues/32

Filed by alpaylan, 2026-11-02. Text as posted:

**A `def temporal` macro whose whole body is `tp(?t)` is rejected as "macro-in-macro is not supported"**

## Symptom

A `def temporal` macro whose body is only a `tp(...)` condition, such as `def temporal at(?t) { tp(?t) };`, makes the whole policy file fail to load with "in `def temporal` body: macro-in-macro is not supported". The body contains no macro call. The same condition written inline loads and evaluates.

## Version

- Dogwood commit: 4ec5e2c01b44ff3882a231ae5d315b330b38fa25 ("Sync language source (2026-10-14) (#20)"). It was still the head of the upstream default branch on 2026-11-02. The crates.io `dogwood-cli` 1.1.0 release and a build of tag v1.0 give the same error.
- dogwood-language 1.1.0 (`dogwood --version`: `dogwood 1.1.0`).
- Built with `cargo build --release -p dogwood-cli` (a debug build gives the same output).

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

Policy `policy.dw`:

```
def temporal at(?t) { tp(?t) };

permit (principal, action == App::Action::"Login", resource);

forbid (principal, action == App::Action::"Login", resource)
when temporal {
    exists (t: Timepoint). formerly within 5s (App::Action::"Login"::request{} && at(t))
};
```

Policy `policy-inlined.dw` (the same policy with the call to `at(t)` replaced by its body `tp(t)`):

```
permit (principal, action == App::Action::"Login", resource);

forbid (principal, action == App::Action::"Login", resource)
when temporal {
    exists (t: Timepoint). formerly within 5s (App::Action::"Login"::request{} && tp(t))
};
```

Event schema: the default (pinned), no `--event-schema`.

Trace `trace.log` (one line):

```
@1 scope(principal: App::User::"alice", resource: App::Gateway::"gw") request_context(input: { user: "alice", n: 1 }) App::Action::"Login"::request(input: { user: "alice", n: 1 }, callerPrincipal: App::User::"alice", callerResource: App::Gateway::"gw", requestId: "r1")
```

## Commands

```
dogwood replay policy.dw --policy-schema schema.cedarschema --trace trace.log
# spec: @1 (time point 0): DENY  [rules: 1]   (same as the inlined policy)
# dogwood: exit 2, no verdicts:
#   × in `def temporal` body: macro-in-macro is not supported
#    ╭─[1:1]
#  1 │ def temporal at(?t) { tp(?t) };
#    · ───┬──
#    ·    ╰── in `def temporal` body: macro-in-macro is not supported
#  2 │ 
#    ╰────
```

```
dogwood replay policy-inlined.dw --policy-schema schema.cedarschema --trace trace.log
# spec: @1 (time point 0): DENY  [rules: 1]
# dogwood: @1 (time point 0): DENY  [rules: 1]   (exit 0)
```

`dogwood validate policy.dw --policy-schema schema.cedarschema` fails with the same error (exit 2). `policy-inlined.dw` validates with no errors.

## Observed and expected verdicts

Decision point @1 (one Login request):

- Observed: none. The policy file is rejected before evaluation (exit 2).
- Expected: DENY. The forbid holds: there is a timepoint `t` within 5s at which a Login request occurred. The inlined policy gives DENY.

The error label also points at the first 6 characters of the file (`def te`) rather than at the body. The span looks body-relative (offset 0, length of `tp(?t)`) and is applied to the whole file.

## Spec text

`dogwood-docs/guide/06-macros.md`, section "`def temporal` — a temporal condition or value":

> Depending on what the body matches, a `def temporal` macro comes in one of two flavours, chosen automatically by the parser:
> - a **temporal condition** — for example a `formerly within ... ...` predicate, which is callable wherever a temporal condition is expected; or
> - a **temporal value** — a `count`/`sum` expression, or arithmetic over such expressions (`(count …) - (count …)`), which is callable only as a comparison operand, never as a standalone condition. A value body must contain at least one aggregation; a bare constant is rejected.

`tp(?t)` is not a `count`/`sum`, so it is a temporal condition.

`dogwood-docs/guide/06-macros.md`, section "No macro-in-macro":

> A macro body may not contain a call to another macro. [...] the temporal equivalent is "in `def temporal` body: macro-in-macro is not supported".

The body `tp(?t)` contains no macro call. `tp` is a reserved name (`RESERVED_MACRO_NAMES` in `dogwood-language/src/macros/mod.rs` lists `"tp"`), so it cannot name a user macro.

`dogwood-docs/guide/04-temporal-expressions.md`, `tp`: "**Syntax:** `tp(t)`" and "`tp(t)` binds `t` to the timepoint currently being evaluated."

The temporal grammar (`dogwood-language/src/extension/temporal/grammar.pest`) has this comment: "`tp_op` and `exists_op` must precede `call` so their keywords (`tp`, `exists`) win — otherwise `tp(t)` would parse as a `call` named \"tp\"." That is what happens in a macro body.

## Engines

The error is raised while the policy file is parsed and lowered (`LoweredPolicySet::from_str` / CLI load), before any temporal engine is chosen. So the default, slice_leaves and partitioned engines cannot be built from this policy at all. The small driver over the public API below prints, for each of the three engines:

```
cargo run --release -- policy.dw schema.cedarschema trace.log
# global: lowering failed: in `def temporal` body: macro-in-macro is not supported
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

Rejected with the same error (`dogwood validate`, trunk):

- body `tp(?t)` (the case above)
- body `(tp(?t))`
- body `tp(t)` and `tp($t)` (macro with no parameters)
- a file that only declares `def temporal at(?t) { tp(?t) };` next to a plain `permit`, with no call to `at` anywhere

Accepted (validate OK):

- body `tp(?t) && App::Action::"Login"::request{}`
- body `App::Action::"Login"::request{} && tp(?t)`. Used as `def temporal login_at(?t) { App::Action::"Login"::request{} && tp(?t) };` with `exists (t: Timepoint). formerly within 5s login_at(t)` in the forbid above, it replays to `@1 (time point 0): DENY  [rules: 1]`.
- body `formerly within 5s tp(?t)`
- body `exists ($t: Timepoint). tp($t)`

Body `!tp(?t)`, when the macro is called, gives a different error ("temporal binding check failed after expansion: existential variable `t` ..."), not this one.

## Suspected code location

- `dogwood-language/src/extension/temporal/parse.rs`, `parse_macro_body`. The rule `temporal_macro_body_entry = { SOI ~ (macro_value_body | condition) ~ EOI }` tries `macro_value_body = { arith ~ &EOI }` first. `agg_expr = { sum_expr | count_expr | call }` lets `tp(?t)` match as a `call` named `tp`, so the whole body parses as a value. `parse_macro_body` then sets `has_agg` for any aggregation operand, including `AggExprKind::Call`, and returns `MacroBody::TemporalValue`. It never reaches its fallback that re-reads an aggregation-free body as a condition.
- `dogwood-language/src/macros/mod.rs`, `check_temporal_agg_body`: the `AggExprKind::Call { .. }` arm reports "in `def temporal` body: macro-in-macro is not supported".

Not counting `AggExprKind::Call` as an aggregation in `parse_macro_body` makes `policy.dw` replay to `@1 (time point 0): DENY  [rules: 1]`. That change alone also changes the error for a value body made only of macro calls (`def temporal c2() { c1() + 1 };` then fails with "a `def temporal` body must be a temporal condition, an aggregation, ..." instead of "macro-in-macro is not supported"), so a fix probably needs to special-case `tp`, for example by treating only a `Call` named `tp` as a condition, or by keeping `agg_expr` from matching `tp(...)`.

## Deny to Allow

Not observed. The policy file is rejected as a whole with an error, so no decision is produced from it. A caller that falls back to an older or smaller policy set when loading fails could lose the forbid, but that depends on the caller, not on Dogwood.

---

Filed as a GitHub issue at the Dogwood maintainers' request.

Found by an automated property-based tester (ephyra); this report was drafted with AI assistance, and every command above was re-run on trunk 4ec5e2c0 and crates.io 1.1.0.
