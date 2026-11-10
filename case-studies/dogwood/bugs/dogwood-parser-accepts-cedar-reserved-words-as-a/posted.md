# Posted: https://github.com/dogwood-policy/dogwood/issues/28

Filed by alpaylan, 2026-11-02. Text as posted:

**Parser accepts Cedar reserved words (is, then, else, in, like, has, true, false) as record keys and attribute names**

Symptom: Dogwood accepts Cedar reserved words as bare record-literal keys, `.attr` accesses and `has` attribute names, for example `{is: 1} has is`. Its guide allows only `if` as a reserved-word key, and Cedar 4.11.0 rejects these with `this identifier is reserved and cannot be used: is`.

Dogwood commit: `4ec5e2c01b44ff3882a231ae5d315b330b38fa25` (upstream main, 2026-10-14, "Sync language source (2026-10-14) (#20)"). It was still the upstream main head on 2026-11-02. dogwood-language version 1.1.0 (workspace version).

Reproduces on trunk: yes (rerun on a build of 4ec5e2c0). The code in question is in `dogwood-language/src/parser/mod.rs`. The crates.io `dogwood-cli` 1.1.0 release and a build of tag v1.0 (996d756d) behave the same: `replay` prints ALLOW.

## Minimal input

`policy.dw` (also valid as `policy.cedar` for the Cedar CLI):

```
permit (principal, action, resource) when { {is: 1} has is };
```

`schema.cedarschema`:

```
namespace Drupe {
  type In = { user: String, n: Long };
  entity Gateway;
  entity OAuthUser;
  action "Read" appliesTo {
    principal: [OAuthUser], resource: [Gateway],
    context: { input: In }
  };
}
```

Event schema: the default one (built-in request/response/error schema).

`trace.log` (one decision event):

```
@1 scope(principal: Drupe::OAuthUser::"bob", resource: Drupe::Gateway::"gw1") request_context(input: { n: 0, user: "bob" }) Drupe::Action::"Read"::request(input: { n: 0, user: "bob" }, callerPrincipal: Drupe::OAuthUser::"bob", callerResource: Drupe::Gateway::"gw1", requestId: "q0")
```

## Commands, observed and expected

```
cedar check-parse --policies policy.cedar
# cedar 4.11.0:
#   × failed to parse policy set
#   ╰─▶ this identifier is reserved and cannot be used: is
#   (the caret points at the `is` after `has`)
```

```
dogwood check-parse policy.dw
# expected: a parse error (the guide allows only `if` as a reserved-word key)
# dogwood (trunk): OK: parsed 1 policy.
```

```
dogwood replay policy.dw --policy-schema schema.cedarschema --trace trace.log
# expected: the policy set fails to parse, no verdict
# dogwood (trunk): @1 (time point 0): ALLOW  [rules: 0]
```

`dogwood validate policy.dw --policy-schema schema.cedarschema` also passes ("OK: validation passed with no errors or warnings.").

`dogwood lower` quotes the names, so the lowered Cedar is valid Cedar:

```
dogwood lower policy.dw --policy-schema schema.cedarschema
# dogwood (trunk), Cedar policies section: permit(principal, action, resource) when { {"is": 1} has "is" };
```

Per decision point:

```
@1  observed: ALLOW
@1  expected: no verdict; the policy set fails to parse
```

## Spec text

`dogwood-docs/guide/02-policy-language.md`, section "Primary expressions: literals, variables, grouping, sets, records", lines 783-784:

> **Record literals** are brace-delimited `key: value` pairs. Keys must be string literals or bare identifiers; as a special case the reserved word `if` is allowed as a key

`dogwood-docs/guide/02-policy-language.md`, section "`has` — attribute existence", lines 637-639:

> The right-hand side may be a dotted path (`has a.b.c`), a string-literal name (`has "attr"`), or (per Cedar RFC 62) the reserved word `if` used as an attribute name (`has if.x`).

`dogwood-syntax/src/grammar.pest`, lines 5-9:

> This grammar describes the Cedar policy language (https://docs.cedarpolicy.com/). It is a pest translation of the upstream LALRPOP grammar at cedar-policy-core/src/parser/grammar.lalrpop (cedar-policy-core v4.10.0).

(The guide, `dogwood-docs/guide/02-policy-language.md` lines 13-15, names v4.11.0 instead. Cedar 4.11.0 is the version checked here.)

Both guide passages name `if` as the only reserved word allowed in these positions. Cedar 4.11.0 rejects every other reserved word there (see Variants).

Cedar 4.11.0 rejects these names in `to_valid_ident` (`cedar-policy-core/src/parser/cst_to_ast.rs`, lines 651-665: `If | True | False | Then | Else | In | Is | Has | Like` → `ReservedIdentifier`), which is used for record keys, `.attr` and `has` names.

## Engines

The default (global), slice_leaves and partitioned engines agree on this input. All three return `@1 ALLOW`. The check used the small Rust driver over the public API below (`Authorizer::new`, `InMemoryTemporalEngine::new().slice_leaves()`, `partition_temporal()`). The bug is in the parser, so every engine gets the same policy.

```
cargo run --release -- policy.dw schema.cedarschema trace.log
global     226.000µs A
slice       42.250µs A
partition   27.542µs A
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

Each line is the condition inside `permit (principal, action, resource) when { ... };`, run through `dogwood check-parse` on trunk and `cedar check-parse` (4.11.0).

These fail (Dogwood accepts, Cedar rejects), for each W in `is`, `then`, `else`, `in`, `like`, `has`, `true`, `false`:

```
{W: 1} == {}
context.W == 1
context has W
context has input.W
{"W": 1}.W == 1
```

The one exception: `context has true` and `context has false` are rejected by both.

These do not fail (both accept): the same five forms with the non-reserved keywords `when` and `principal`.

Related, not part of this bug:

- `if`: Dogwood accepts all five forms and Cedar 4.11.0 rejects them. The guide documents `{ if: ... }` and `has if.x` as a special case.
- `__cedar`: Dogwood accepts all five forms. Cedar 4.11.0 rejects them with a different message: "The name `__cedar` contains `__cedar`, which is reserved".

## Suspected code location

`dogwood-language/src/parser/mod.rs`:

- `record_key`: accepts any single-token key text as a bare key.
- `field_name`: returns the `any_ident` of a `.attr` access with no reserved-word check. `build_member`, `extract_has_path` and `extract_attr_path` use it.
- `extract_attr_path`: takes the bare `name` on the right of `has` with no reserved-word check.

`dogwood-syntax/src/grammar.pest` lines 23-27 admit keywords in these positions on purpose and leave the real rules to the parser ("`{ if: 1 }` is a valid record literal key"). No later pass rejects the reserved words.

A reserved-word check in these three functions (keeping the documented `if` exception) would match Cedar's behaviour. The renderer already quotes reserved names when lowering.

## Can it flip a Deny to an Allow?

Not through evaluation. Dogwood gives the accepted policy its plain meaning: `{is: 1} has is` is true, and `dogwood lower` emits the equivalent valid Cedar `{"is": 1} has "is"`. A forbid version of the same condition (`forbid ... when { {is: 1} has is };` next to a blanket `permit (principal, action, resource);`) gave `@1 DENY` on all three engines, as intended. The effect is that Dogwood loads and runs policy text that its guide and Cedar tooling reject. In the example above, a policy set that should fail to load returns ALLOW instead. Only the policy author can trigger this; event data cannot.

---

Filed as a GitHub issue at the Dogwood maintainers' request.

Found by an automated property-based tester (ephyra); this report was drafted with AI assistance, and every command above was re-run on trunk 4ec5e2c0 and crates.io 1.1.0.
