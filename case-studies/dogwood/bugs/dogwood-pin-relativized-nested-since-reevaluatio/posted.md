# Posted: https://github.com/dogwood-policy/dogwood/issues/29

Filed by alpaylan, 2026-11-02. Text as posted:

**Nested positive-left `since` under the default pinned event schema takes time polynomial in the number of in-window events, with the degree growing with nesting depth**

## Symptom

Take a policy with a positive-left `since` nested inside another `since`, and the default event schema (callerPrincipal pin).
Replaying 12 requests from one principal, 1 s apart, takes about 11 s, and 16 take about 63 s.
The same policy and trace take 10-20 ms with the built-in unpinned schema, and under 1 ms in the partitioned engine.
The verdicts are correct. Only the evaluation time is wrong.

This report covers one trigger (nested positive-left `since`) of the cause described in our report emailed on 2026-10-28, "Nested `previous` / `since` under the default (pinned) event schema take time polynomial in the number of in-window events, with the degree growing with nesting depth". The other trigger, nested `previous`, is in #30. In both, the L-Relativize rewrite places the rewritten inner operand into the encoding more than once per nesting level (three times for `since`, twice for `previous`), and `interpreter/eval.rs` evaluates each copy again.

## Version

- Dogwood commit: 4ec5e2c01b44ff3882a231ae5d315b330b38fa25 ("Sync language source (2026-10-14) (#20)"). This was still the upstream default branch head on 2026-11-02. The crates.io `dogwood-cli` 1.1.0 release and tag v1.0 show the same timings.
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
    App::Action::"Write"::request{} since within 1m (App::Action::"Write"::request{} since within 1m App::Action::"Write"::request{})
};
```

Event schema: the default (no `--event-schema`).

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
# expected (spec): finishes quickly (the unpinned run below takes 12 ms); @1..@16 ALLOW
# dogwood 4ec5e2c0: @1..@16 ALLOW after about 63 s
```

The same command with the unpinned schema shipped in the repo (`dogwood-language/configuration/event-schemas/unpinned.dwschema`, copied next to the other files):

```
dogwood replay policy.dw --policy-schema schema.cedarschema --trace trace.log --event-schema unpinned.dwschema
# dogwood 4ec5e2c0: @1..@16 ALLOW in about 12 ms
```

With a single principal, the pinned and unpinned schemas must give the same verdicts, because every event agrees with the request on callerPrincipal.

## Observed and expected verdicts per decision point

```
@1..@16 ALLOW
# expected: the same (each event is a Write within 1m, so the anchor can be the decision point itself)
```

At every N below, the default schema gives the same output as the unpinned schema; I compared the outputs. Wall time for the default schema (`dogwood replay`, release build, one run each, idle Apple-silicon machine):

```
N=8   0.95 s
N=10  3.50 s
N=12  10.57 s
N=14  27.65 s
N=16  63.08 s
```

The growth is polynomial in N, not exponential: the ratio for each extra 2 events falls (3.7, 3.0, 2.6, 2.3), and the local log-log slope is about 6. One more nesting level raises it to about 8 (depth 3: N=4 0.88 s, N=5 4.7 s, N=6 19.4 s, N=7 66 s). So the degree of the polynomial grows with the nesting depth.

## Spec text

dogwood-docs/guide/04-temporal-expressions.md, "`since` — held continuously since an anchor":

> Formally it holds at the decision point when there is an anchor timepoint `j` in the window where `right` held, and `left` held at **every** step from `j+1` through the decision point.

dogwood-docs/guide/03-event-schema.md, "Universal symmetric pins: key-local semantics and the partition guarantee":

> **The positive left of `since` quantifies over this key's positions only.**

> This is implemented by a lowering-time rewrite of the leaf formulas [...] The default event schema carries such a pin, on `callerPrincipal`, so key-local semantics apply unless you replace it

dogwood-docs/guide/08-formal-specification.md (L-Relativize):

> replaces a `since` whose left is not a μ-confined negation by a count-equality encoding of "every μ-position in the anchor range satisfies the left".

> The default event schema is not such a case: it pins `callerPrincipal` on every derived kind, so under the default the rewrite is active.

dogwood-language/src/event_schema/relativize.rs, module doc:

> Partitioned storage then implements the same semantics at zero cost: within a partition every event is "mine", so the guards are tautological.

None of these documents states a cost bound. The expectation is ours: the rewrite only changes which events a leaf sees. It should not turn a 16-event replay into a computation that runs for about a minute when the unrewritten form takes milliseconds.

## Engines

I ran the small Rust program below against the public API (`Authorizer::new`; `Authorizer::builder(..).temporal_engine(InMemoryTemporalEngine::new().slice_leaves())`; `Authorizer::builder(..).partition_temporal()`). Same policy and schema, default event schema, 12 events (`bash gen-trace.sh 12 > t12.log`), at 4ec5e2c0:

```
cargo run --release -- policy.dw schema.cedarschema t12.log
```

Result: global 10.65 s, slice 10.74 s, partition 0.25 ms. All three print `AAAAAAAAAAAA` (12 Allows).

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

## Variants (default schema, N=16, 1 s apart, window 1m unless noted)

W below is `App::Action::"Write"::request{}`; each variant replaces the condition in `policy.dw`.

These fail (slow):

- The repro (depth 2): 63 s at N=16.
- Depth 3, `W since within 1m (W since within 1m (W since within 1m W))`: 19 s at N=6, 66 s at N=7.
- Nesting on the left side, `(W since within 1m W) since within 1m W`: 2.5 s.
- The repro with window `10s` instead of `1m`: 32 s (fewer events fall inside the window).

These do not fail (fast, under 0.2 s on trunk):

- The same policy with the unpinned schema: 12 ms.
- A single `W since within 1m W`: 46 ms.
- The negated-left idiom nested, `!App::Action::"Login"::request{} since within 1m (!App::Action::"Login"::request{} since within 1m W)`: 17 ms. This idiom takes the rewrite's fast path, so it is not count-encoded.
- `formerly within 1m (formerly within 1m W)`: 17 ms.
- The repro with events 30 s apart (`bash gen-trace.sh 16 | awk '{ $1 = "@" (NR*30-29); print }'`, giving `@1`, `@31`, `@61`, ...), so at most 2 fall inside the window: 122 ms.

So the cost grows with the number of same-key events in the window and with the nesting depth of positive-left `since`.

## Suspected code location

- `dogwood-language/src/event_schema/relativize.rs`, `rewrite_since` (general encoding). Under a universal pin, `L since[0,W] A` becomes `exists ti, tj, n1, n2` over two `count` aggregates. Their bodies are `formerly[0,W](μ ∧ tp(tk)) ∧ range` and `formerly[0,W]((μ ∧ L') ∧ tp(tk)) ∧ range`. The anchor `formerly[0,W](A' ∧ tp(tj))` is cloned three times: into the outer body and into both count ranges. When `A` or `L` is itself a `since`, each copy contains the whole inner encoding. The number of `Formerly` nodes in the rewritten leaf is 5, 20, 65, 200 at depths 1-4 (3·f(d-1)+5).
- `dogwood-language/src/interpreter/eval.rs`, `eval_condition` and `match_occurrences` (and `scan_formerly` / `eval_since`, which call them). Nothing is memoised. The inner `formerly((μ ∧ L') ∧ tp(tk))` mentions neither `tj` nor `ti`, yet it is evaluated again at the same timepoint for every candidate binding of `tj`/`ti`. Every cloned anchor is evaluated again on its own. Each nesting level multiplies the work by a power of the number of in-window events.

One possible direction: memoise temporal sub-formula results within one top-level `eval_condition` call, keyed by the structural node, the timepoint and the bindings of the node's own variables, leaving the rewrite and the leaves unchanged. No patch is attached, and this report does not include measurements of such a change.

## Deny to Allow

I saw no verdict change. Every run matched the unpinned schema, and the three engines agreed. The bug cannot flip a Deny to an Allow within Dogwood. A caller that puts a deadline on `is_authorized` and fails open on timeout could turn the delay into an Allow, but that depends on the caller. The cost is driven by the requesting principal's own events inside the window, so one principal can slow down its own decisions. The README says the reference interpreter is "**NOT** intended to be used directly as an authorization engine".

---

Filed as a GitHub issue at the Dogwood maintainers' request.

Found by an automated property-based tester (ephyra); this report was drafted with AI assistance, and every command above was re-run on trunk 4ec5e2c0 and crates.io 1.1.0.
