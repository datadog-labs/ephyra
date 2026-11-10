# Posted: https://github.com/dogwood-policy/dogwood/issues/31

Filed by alpaylan, 2026-11-02. Text as posted:

**Provider record field names that are not Cedar identifiers ("a b", "x-y", "if") are written unquoted into a synthesized schema, so lowering fails**

Symptom: a `providers.json` whose record `outputType` has a field named `"a b"`, `"x-y"` or a Cedar reserved word (`"if"`, `"has"`, `"is"`) passes `dogwood schema providers`. But every policy set that calls the provider fails to lower with `synthesize providers type: error parsing schema: unexpected token ...`, and `validate`, `lower` and `replay` give no verdict.

Dogwood commit: `4ec5e2c01b44ff3882a231ae5d315b330b38fa25` (upstream main, 2026-10-14, "Sync language source (2026-10-14) (#20)"). It was still the upstream main head on 2026-11-02. dogwood-language version 1.1.0 (`dogwood --version`: `dogwood 1.1.0`).

Reproduces on trunk: yes (rerun on a build of 4ec5e2c0). The same unquoted `format!("{name}{opt}: ...")` line is at tag v1.0. A build of v1.0 and the crates.io `dogwood-cli` 1.1.0 release give the same error.

## Minimal input

`providers.json`:

```
{"availableProviders":{"Odd::Keys":{"argumentTypes":[],"outputType":{"paramType":"record","fields":{"a b":{"paramType":"long"}},"required":["a b"]},"implementation":{"kind":"rhai","script":"fn evaluate() { #{ \"a b\": 1 } }"}}}}
```

`policy.dw`:

```
permit (principal, action, resource) when { Odd::Keys()["a b"] < context.input.n };
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

`trace.log` (one decision event, `n: 5`):

```
@1 scope(principal: Drupe::OAuthUser::"bob", resource: Drupe::Gateway::"gw1") request_context(input: { n: 5, user: "bob" }) Drupe::Action::"Read"::request(input: { n: 5, user: "bob" }, callerPrincipal: Drupe::OAuthUser::"bob", callerResource: Drupe::Gateway::"gw1", requestId: "q0")
```

## Commands, observed and expected

```
dogwood schema providers providers.json
# dogwood (trunk): OK: providers schema is valid.
```

```
dogwood replay policy.dw --policy-schema schema.cedarschema --providers providers.json --trace trace.log
# expected: @1 (time point 0): ALLOW  [rules: 0]   (the provider returns 1, and 1 < 5)
# dogwood (trunk):
#   × provider schema augmentation failed: synthesize providers type: error
#   │ parsing schema: unexpected token `b` (from `type __PR = { policy_0_p_0:
#   │ { a b: Long } };`)
```

`dogwood validate` and `dogwood lower` with the same arguments print the same error.

Cedar's schema syntax accepts the field name when it is quoted, and rejects it unquoted:

```
cedar translate-schema --direction cedar-to-json --schema q.cedarschema   # q: type T = { "a b": Long, "x-y"?: Long, "if": Long };
# cedar 4.11.0: {"":{"commonTypes":{"T":{"type":"Record","attributes":{"a b":...,"if":...,"x-y":{...,"required":false}}}},...}}
cedar translate-schema --direction cedar-to-json --schema u.cedarschema   # u: type T = { a b: Long };
# cedar 4.11.0: × error parsing schema: unexpected token `b`  (expected `:` or `?`)
```

Per decision point:

```
@1  observed: no verdict; the policy set fails to lower
@1  expected: ALLOW
```

## Spec text

`dogwood-docs/guide/10-provider-schema.md`, section "`paramType` variants and their Cedar types":

> For a `record`, each field is rendered as `name: T` if `name` is listed in `required`, otherwise `name?: T` (optional). Records and sets nest.

The guide puts no limit on record field names. `dogwood schema providers` accepts the file above, and `dogwood check-parse policy.dw --providers providers.json` accepts the policy ("OK: parsed 1 policy. policy 0: 1 provider call(s)").

Same file, section "How a provider binds to `context.providers.<id>`":

> **At lowering time**, every provider invocation is hoisted. [...] Dogwood also **augments the Cedar schema**. It defines one generated common type per touched namespace [...] containing every hoisted provider field typed from its provider's declared `outputType`.

A declaration that `dogwood schema providers` accepts should not make lowering fail with an internal schema-parse error.

## Engines

The failure happens while lowering (`LoweredPolicySet::from_str`), before any `Authorizer` is built. So the default, slice_leaves and partitioned engines all get the same error. The small driver over the public API below prints, for each of the three engines:

```
cargo run --release -- policy.dw schema.cedarschema trace.log providers.json
# global: lowering failed: provider schema augmentation failed: synthesize providers type: error parsing schema: unexpected token `b` (from `type __PR = { policy_0_p_0: { a b: Long } };`)
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

Fail on trunk (same error, with the token named):

```
field "a b"                       -> unexpected token `b`
field "x-y"                       -> invalid token
field "if" / "has" / "is"         -> `if` is a reserved identifier (and so on)
fields "ok" and "a b", policy reads only ["ok"] or .ok   -> unexpected token `b`
permit (principal, action, resource);
forbid (principal, action, resource) when { Odd::Keys()["a b"] < context.input.n };
                                  -> whole policy set fails to lower, no verdict
```

Do not fail:

```
field "ok" only, policy reads ["ok"]                -> ALLOW
fields "ok" and "a b", provider never called (when { true })  -> ALLOW
```

So the bug needs a provider call. It does not matter which field the policy reads. The provider's whole output type is rendered.

## Suspected code location

`dogwood-language/src/extension/provider/declarations.rs`, `ParamType::to_cedar_type_with`, `"record"` arm (line 623 at 4ec5e2c0):

```
attrs.push(format!("{name}{opt}: {}", ty.to_cedar_type_with(names)));
```

The field name is written as-is. `dogwood-language/src/cedarify/schema_augment.rs`, `add_provider_context_fields`, builds `type __PR = { ... };` from that string and parses it with Cedar's schema parser (lines 650-664), and the parser rejects the name. The provider declaration checks (`dogwood schema providers`) do not check field names.

Quoting the field name in that `format!` (for example, always writing it as a quoted string, `format!("{:?}{opt}: {}", name, ...)`) is enough. On an otherwise unchanged 4ec5e2c0, the repro then prints `@1 (time point 0): ALLOW  [rules: 0]`, and the forbid variant prints `@1 (time point 0): DENY  [rules: 1]`. `lower` emits `(((context.providers).policy_0_p_0)["a b"]) < ((context.input).n)`.

## Can it flip a Deny to an Allow?

No such case was found. The whole policy set fails to load, so no decision is returned at all. In the forbid variant above, trunk returns no verdict rather than Allow. A service that loads such a set gets a load error. What it does next is up to the service and was not tested.

---

Filed as a GitHub issue at the Dogwood maintainers' request.

Found by an automated property-based tester (ephyra); this report was drafted with AI assistance, and every command above was re-run on trunk 4ec5e2c0 and crates.io 1.1.0.
