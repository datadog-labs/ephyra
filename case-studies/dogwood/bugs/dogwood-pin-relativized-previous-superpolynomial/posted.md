# Posted: https://github.com/dogwood-policy/dogwood/issues/30

Filed by alpaylan, 2026-11-02. Text as posted:

**Nested `previous` under the default pinned event schema takes time polynomial in the number of in-window events, with the degree growing with nesting depth**

## Symptom

Take a policy with `previous within 1m (previous within 1m (...))` and the default event schema (callerPrincipal pin).
Replaying 16 requests from one principal, 1 s apart, takes about 19 s, and 20 take about 85 s.
The same policy and trace take 10-20 ms with the built-in unpinned schema, and under 1 ms in the partitioned engine.
The verdicts are correct. Only the evaluation time is wrong.

This report covers one trigger (nested `previous`) of the cause described in our report emailed on 2026-10-28, "Nested `previous` / `since` under the default (pinned) event schema take time polynomial in the number of in-window events, with the degree growing with nesting depth". The other trigger, nested positive-left `since`, is in #29. In both, the L-Relativize rewrite places the rewritten inner operand into the encoding more than once per nesting level (twice for `previous`, three times for `since`), and `interpreter/eval.rs` evaluates each copy again.

## Version

- Dogwood commit: 4ec5e2c01b44ff3882a231ae5d315b330b38fa25 ("Sync language source (2026-10-14) (#20)"). It was still the upstream default branch head on 2026-11-02. The crates.io `dogwood-cli` 1.1.0 release and tag v1.0 show the same timings.
- dogwood-language 1.1.0 (workspace version).
- Built with `cargo build --release -p dogwood-cli`.

## Minimal input

Action schema `schema.cedarschema`:

```
namespace App {
  type In = { user: String };
  entity Gateway;
  entity User = { id: String };
  action "Login" appliesTo {
    principal: [User], resource: [Gateway],
    context: { input: In }
  };
  action "Write" appliesTo {
    principal: [User], resource: [Gateway],
    context: { input: In }
  };
}
```

Policy `policy.dw`:

```
permit (principal, action == App::Action::"Write", resource)
when temporal {
    previous within 1m (previous within 1m (!App::Action::"Login"::response{ input.user: "zed" }))
};
```

Event schema: the default (no `--event-schema`). No event carries user "zed", so the inner negation holds at every event.

Trace `trace.log`: `bash gen-trace.sh 16 > trace.log`. Each line looks like this one:

```
@1 scope(principal: App::User::"alice", resource: App::Gateway::"gw") request_context(input: { user: "alice" }) App::Action::"Write"::request(input: { user: "alice" }, callerPrincipal: App::User::"alice", callerResource: App::Gateway::"gw", requestId: "r1")
```

`gen-trace.sh N` prints N `Write` requests from principal alice, at `@1` through `@N` (1 s apart):

```bash
#!/usr/bin/env bash
N=${1:-12}
for i in $(seq 1 "$N"); do
  echo "@$i scope(principal: App::User::\"alice\", resource: App::Gateway::\"gw\") request_context(input: { user: \"alice\" }) App::Action::\"Write\"::request(input: { user: \"alice\" }, callerPrincipal: App::User::\"alice\", callerResource: App::Gateway::\"gw\", requestId: \"r$i\")"
done
```

## Command

```
dogwood replay policy.dw --policy-schema schema.cedarschema --trace trace.log
# expected (spec): finishes quickly (the unpinned run below takes 10-20 ms); @1 DENY, @2 DENY, @3..@16 ALLOW
# dogwood 4ec5e2c0: same verdicts, after about 19 s wall time
```

The same command with the unpinned schema shipped in the repo (`dogwood-language/configuration/event-schemas/unpinned.dwschema`, copied next to the other files):

```
dogwood replay policy.dw --policy-schema schema.cedarschema --trace trace.log --event-schema unpinned.dwschema
# dogwood 4ec5e2c0: same verdicts in 10-20 ms
```

With a single principal, the pinned and unpinned schemas must give the same verdicts, because every event agrees with the request on callerPrincipal.

## Observed and expected verdicts per decision point

The verdicts are the same on every run, and the output matches the unpinned run at every N below. Only the time differs.

```
@1 DENY      # no previous event
@2 DENY      # previous event @1 has no previous event
@3..@16 ALLOW
# expected: the same
```

Wall time for the default schema (`dogwood replay`, release build, one run each, idle Apple-silicon machine):

```
N=8   0.31 s
N=12  2.9 s
N=14  7.7 s
N=16  19.0 s
N=18  41.8 s
N=20  84.9 s
```

The growth is polynomial in N, not exponential: the ratio for each extra 2 events falls (2.7, 2.5, 2.2, 2.0), and the local log-log slope is about 6.5. One more nesting level raises the degree (depth 3: 12.1 s at N=8, versus 0.31 s at depth 2).

## Spec text

dogwood-docs/guide/04-temporal-expressions.md, "`previous` — the immediately preceding event":

> `previous` is stricter than `formerly`: it looks only at the **immediately preceding timepoint** (`i - 1`), not the whole window.

dogwood-docs/guide/03-event-schema.md, "Universal symmetric pins":

> **`previous` means "this key's previous event."**

> This is implemented by a lowering-time rewrite of the leaf formulas [...] The default event schema carries such a pin, on `callerPrincipal`, so key-local semantics apply unless you replace it

dogwood-docs/guide/08-formal-specification.md (L-Relativize):

> The default event schema is not such a case: it pins `callerPrincipal` on every derived kind, so under the default the rewrite is active.

dogwood-language/src/event_schema/relativize.rs, module doc:

> Partitioned storage then implements the same semantics at zero cost: within a partition every event is "mine", so the guards are tautological.

None of these documents says anything about evaluation cost. The expectation is ours: the rewrite only changes which events a leaf sees. It should not turn a 16-event replay into a 19 s computation when the unrewritten form takes milliseconds.

## Engines

I ran the small Rust program below against the public API (`Authorizer::new`; `Authorizer::builder(..).temporal_engine(InMemoryTemporalEngine::new().slice_leaves())`; `Authorizer::builder(..).partition_temporal()`). Same policy and schema, default event schema, 12 events (`bash gen-trace.sh 12 > t12.log`):

```
cargo run --release -- policy.dw schema.cedarschema t12.log
```

Result: global 2.86 s, slice 2.83 s, partition 0.26 ms. All three print `DDAAAAAAAAAA`.

The global (default) and slice_leaves engines are both slow. The partitioned engine is not; per the spec it evaluates the non-relativized leaves.

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

## Variants (default schema unless noted, N=16, 1 s apart, window 1m)

These fail (slow):

- Nested `previous` over a negated predicate (the repro): 19 s.
- Nested `previous` over a positive predicate, `previous within 1m (previous within 1m (App::Action::"Write"::request{ input.user: "alice" }))`: 13 s.
- The repro with window `1h` instead of `1m`, N=12: 2.9 s (the same as `1m`).
- Depth 3, `previous within 1m (previous within 1m (previous within 1m (!App::Action::"Login"::response{ input.user: "zed" })))`, N=8: 12.1 s.

These do not fail (fast):

- The same nested policy with the unpinned schema: 10-20 ms.
- A single `previous within 1m (!...)`: 39 ms.
- `formerly within 1m (formerly within 1m (!...))`: 16 ms.
- The repro with 20 events spaced 30 s apart (`bash gen-trace.sh 20 | awk '{ $1 = "@" (NR*30-29); print }'`), so at most 3 fall inside the 1m window: 33 ms.

So the cost grows with the number of same-key events inside the window and with the `previous` nesting depth.

## Suspected code location

- `dogwood-language/src/event_schema/relativize.rs`, `rewrite_previous` and `rewrite_since`. Under a universal pin, `previous[0,W] B` becomes an anti-join over `tp`-bound positions. Inside a negated `exists`, the encoding re-derives the anchor as `formerly[0,W](B && tp(tj))`, and the same anchor is used again outside it. `rewrite_since` uses the same re-derived anchor inside count bodies. So `B` itself (here, the inner `previous`) is evaluated again at every in-window position for every candidate. The number of `Formerly` nodes in the rewritten leaf is 3, 9, 21, 45 at depths 1-4 (2·f(d-1)+3).
- `dogwood-language/src/interpreter/eval.rs`:
  - `scan_formerly`, `eval_since` and the `Formerly`/`Since` arms of `match_occurrences` evaluate the body at every in-window position, even when a `tp(tj)` conjunct with `tj` already bound means only one position can match.
  - The `And` arm of `match_occurrences` enumerates conjuncts left to right. Cheap filters such as `tj < tk` run only after the costly re-derivation.
  - The `Exists` arm of `eval_condition` builds every row when only emptiness matters.
  - Nothing is memoised, so each copy of the anchor is evaluated on its own.

Each nesting level multiplies the work.

Possible directions: in `eval.rs`, skip positions excluded by a bound `tp`, apply ready comparison filters early, and stop `exists` at the first row; or memoise temporal sub-formula results within one top-level `eval_condition` call, which would also cover the `since` trigger. No patch is attached, and this report does not include measurements of such changes.

## Deny to Allow

I saw no verdict change: every run gave the same verdicts as the unpinned schema and the partitioned engine. The bug cannot flip a Deny to an Allow within Dogwood. A caller that puts a deadline on `is_authorized` and fails open on timeout could turn the delay into an Allow, but that depends on the caller. The cost is driven by the caller's own request count inside the window, so a single principal can make its own decisions slow. The README says the reference interpreter is "**NOT** intended to be used directly as an authorization engine".

---

Filed as a GitHub issue at the Dogwood maintainers' request.

Found by an automated property-based tester (ephyra); this report was drafted with AI assistance, and every command above was re-run on trunk 4ec5e2c0 and crates.io 1.1.0.
