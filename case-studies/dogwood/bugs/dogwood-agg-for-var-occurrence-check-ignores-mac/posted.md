# Posted: https://github.com/dogwood-policy/dogwood/issues/21

Filed by alpaylan, 2026-11-02. Text as posted:

**`count`/`sum` rejected with "`for`-variable does not occur in the aggregation body" when the variable is used only inside a temporal macro call**

## Symptom

A `count` or `sum` whose `for` variable is used in the `where` body only as an argument to a `def temporal` macro call is rejected at parse time with "`for`-variable `u` does not occur in the aggregation body". The same policy with the macro call written out by hand is accepted.

## Version

- Dogwood main 4ec5e2c01b44ff3882a231ae5d315b330b38fa25 (tag `v1.1`, "Sync language source (2026-10-14) (#20)"), still the head of the default branch on 2026-11-02.
- crates.io `dogwood-cli` 1.1.0 gives the same output.

## Minimal input

Policy `policy.dw`:

```
def temporal login_by(?u) { App::Action::"Login"::request{ input.user: ?u } };

permit (principal, action == App::Action::"Login", resource)
when temporal {
    (count for (u: String). where (formerly within 1h login_by(u))) < 3
};
```

Control `policy-inlined.dw` (the call `login_by(u)` replaced by its body):

```
permit (principal, action == App::Action::"Login", resource)
when temporal {
    (count for (u: String). where (formerly within 1h App::Action::"Login"::request{ input.user: u })) < 3
};
```

Action schema `schema.cedarschema`:

```
namespace App {
  type In = { user: String };
  entity Gateway;
  entity User;
  action "Login" appliesTo {
    principal: [User], resource: [Gateway],
    context: { input: In }
  };
}
```

Event schema: the default (pinned), no `--event-schema`. No providers.

Trace `trace.log` (three Login requests by the same principal, `input.user` = a, b, c):

```
@1 scope(principal: App::User::"alice", resource: App::Gateway::"gw") request_context(input: { user: "a" }) App::Action::"Login"::request(input: { user: "a" }, callerPrincipal: App::User::"alice", callerResource: App::Gateway::"gw", requestId: "r1")
@2 scope(principal: App::User::"alice", resource: App::Gateway::"gw") request_context(input: { user: "b" }) App::Action::"Login"::request(input: { user: "b" }, callerPrincipal: App::User::"alice", callerResource: App::Gateway::"gw", requestId: "r2")
@3 scope(principal: App::User::"alice", resource: App::Gateway::"gw") request_context(input: { user: "c" }) App::Action::"Login"::request(input: { user: "c" }, callerPrincipal: App::User::"alice", callerResource: App::Gateway::"gw", requestId: "r3")
```

## Commands

```
dogwood validate policy.dw --policy-schema schema.cedarschema
# spec: OK (the expanded policy is the accepted policy-inlined.dw)
# dogwood (exit 2):
#     × `for`-variable `u` does not occur in the aggregation body (u); a group-by
#     │ variable must be used in the `where` body, otherwise the grouping ranges
#     │ over an unbounded domain
#      ╭─[5:5]
#    4 │ when temporal {
#    5 │     (count for (u: String). where (formerly within 1h login_by(u))) < 3
#      ·     ──────────────────────────────────┬─────────────────────────────────
#      ·                                       ╰── `for`-variable `u` does not occur in the aggregation body (u); a group-by variable must be used in the `where` body, otherwise the grouping ranges over an unbounded domain
#    6 │ };
#      ╰────
```

```
dogwood replay policy.dw --policy-schema schema.cedarschema --trace trace.log
# spec:    @1 ALLOW, @2 ALLOW, @3 DENY (same as policy-inlined.dw)
# dogwood: same error as above, no verdicts (exit 2)
```

Control:

```
dogwood validate policy-inlined.dw --policy-schema schema.cedarschema
# spec:    OK
# dogwood: OK: validation passed with no errors or warnings.
dogwood replay policy-inlined.dw --policy-schema schema.cedarschema --trace trace.log
# spec:    @1 ALLOW, @2 ALLOW, @3 DENY
# dogwood: @1 (time point 0): ALLOW  [rules: 0]
#          @2 (time point 1): ALLOW  [rules: 0]
#          @3 (time point 2): DENY
```

Per decision point: expected @1 ALLOW, @2 ALLOW, @3 DENY (at @3 the distinct users within 1h are a, b, c, so `count < 3` is false and no permit applies). Observed: the policy set does not load, so there is no verdict at any decision point.

## Expected behaviour (spec)

`dogwood-docs/guide/06-macros.md`, §"What a macro is (and is not)", lines 19-24:

> You declare a macro once at the top of a `.dw` file, and everywhere you call it expansion splices the macro body in — substituting the call arguments — before the policy is lowered. By the time the temporal evaluator or the Cedar backend sees your policy, there are no macro definitions and no calls left; the definitions have been consumed and every call has been replaced by its expanded body.

`dogwood-docs/guide/08-formal-specification.md`, §5.4 "Temporal acceptance (well-formedness)", lines 729-731:

> All are **conservative** ("reject if unsure") and are **suspended** in the presence of a macro sigil (re-checked post-expansion — **WF-Sigil-Punt**).

The occurrence rule itself, §5.4 (WF-Agg-Domain-Occurs), lines 870-873: "Every `for` variable must **occur** in the body … Otherwise: *"`for`-variable `g` does not occur in the aggregation body …"*." After expansion `u` does occur in the body, as the field `input.user: u`; `policy-inlined.dw` is that expanded form and is accepted.

The code's own contract, `dogwood-language/src/extension/temporal/check.rs:195-198` (doc of `check_exists_safe`):

> A macro sigil (`?s` SigilRef / unresolved `Call`) makes the body opaque — its concrete atoms exist only after expansion — so we accept here and let the post-expansion re-check enforce safety on the substituted form (mirrors `check_aggregation`'s sigil punt).

`exists (u: String). login_by(u)` is accepted on trunk (variant v8), so the `exists` check does punt on the call. The aggregation check does not.

## Engines

The policy set fails at parse time, before any engine is chosen: `LoweredPolicySet::from_str` returns the same error. For `policy-inlined.dw`, the default engine, `InMemoryTemporalEngine::new().slice_leaves()` and `partition_temporal()` all give @1 Allow, @2 Allow, @3 Deny.

All three engines agree on the inlined control. All three refuse the macro form, since it never reaches them.

## Variants (all re-run with `dogwood validate` on trunk 4ec5e2c0 and 1.1.0)

Each variant uses the schema above with `type In = { user: String, n: Long };`, and defines

```
def temporal login_by(?u) { App::Action::"Login"::request{ input.user: ?u } };
def temporal login_n(?k) { App::Action::"Login"::request{ input.n: ?k } };
def temporal login() { App::Action::"Login"::request{} };
def temporal login_free() { App::Action::"Login"::request{ input.user: u } };
```

and the condition below.

Rejected with "does not occur in the aggregation body" (bug):
- v1 `(count for (u: String). where (formerly within 1h login_by(u))) < 3` (the PoC)
- v2 `(count for (u: String), (t: Timepoint). where (login_by(u) && tp(t))) < 3`
- v3 `(count for (u: String), (t: Timepoint). where (formerly within 1h (login_by(u) && tp(t)))) < 3`
- v4 `(sum k for (k: Long). where (formerly within 1h login_n(k))) < 3` (`sum` is affected too)

Accepted (no bug):
- v7 `count_distinct_within(u, 1h, login_by(u)) < 3`: the stdlib macro puts a sigil `?k` in the `for` list, and that case is punted.
- v8 `exists (u: String). login_by(u)`: the `exists` check punts on the call.
- v9 the inlined form of v1.
- v10 `(count for (u: String). where (formerly within 1h (login() && App::Action::"Login"::request{ input.user: u }))) < 3`: `u` also occurs outside the call.

Rejected, correctly:
- v6 `(count for (u: String). where (formerly within 1h login())) < 3`: `u` does not occur even after expansion.

Unclear:
- v5 `(count for (u: String). where (formerly within 1h login_free())) < 3`: `u` appears in the macro body as a plain name, not a parameter. Trunk rejects it with the same error. I did not find a doc sentence that says whether a plain name in a macro body may refer to a binder at the call site, so this report does not claim v5 is a bug.

## Suspected code location

`dogwood-language/src/extension/temporal/check.rs`, function `check_aggregation` (line 670). It runs at parse time through `Temporal::parse` -> `check_condition` (`extension/temporal/mod.rs:37`). The occurrence check ("`for`-variable … does not occur", line 745) and the range-restriction check after it read the body's variables with `collect_free` / `collect_range_restricted`, which treat an unresolved macro `Call` as opaque. So a `for` variable used only inside a call looks absent. `check_aggregation` punts only when the `for` list itself holds a sigil. `check_exists_safe` (line 199) punts when `body_has_sigil(body)` (line 601) holds. The post-expansion re-run (`macros/mod.rs:639`, `check_condition` on the expanded condition) would still catch a `for` variable that is really unused, as in v6.

## Can it flip a Deny to an Allow?

Not directly. The error makes the whole policy set fail to load, so there are no verdicts. In the PoC the rejected policy is a `permit`; the same rejection applies to a `forbid` written this way. Such a `forbid` cannot be loaded. An author who works around the error by deleting the aggregation would weaken the policy, but that is an authoring choice, not something the bug does on its own.

Filed as a GitHub issue at the Dogwood maintainers' request.

Found by an automated property-based tester (ephyra); this report was drafted with AI assistance, and every command above was re-run on trunk 4ec5e2c0 and crates.io 1.1.0.
