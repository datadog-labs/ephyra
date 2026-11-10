# Posted: https://github.com/dogwood-policy/dogwood/issues/26

Filed by alpaylan, 2026-11-02. Text as posted:

**`dogwood explain` prints `context has input.user` and `context has "input.user"` as the same text, but the two conditions decide differently**

Symptom: `dogwood explain` (`ParsedPolicySet::informalize`) joins the segments of a `has` path with `.` and does not quote them. So `has input.user` (a path of two attributes) and `has "input.user"` (one attribute whose name contains a dot) both read "has input.user", but they test different things.

Version: Dogwood main 4ec5e2c01b44ff3882a231ae5d315b330b38fa25 (tag `v1.1`), still the tip of the default branch on 2026-11-02, and crates.io `dogwood-cli` 1.1.0 (same output).

## Minimal input

`policy-path.dw`:

```
permit (principal, action, resource)
when { context has input.user };
```

`policy-dotted.dw`:

```
permit (principal, action, resource)
when { context has "input.user" };
```

`schema.cedarschema` (action schema):

```
entity User;
entity Doc;
action "Read" appliesTo {
  principal: [User], resource: [Doc],
  context: { input: { user?: String } }
};
```

Event schema: the default (no `--event-schema`).

`trace.log`:

```
@0 scope(principal: User::"u", resource: Doc::"d") request_context(input: { user: "alice" }) Action::"Read"::request(input: { user: "alice" }, callerPrincipal: User::"u", callerResource: Doc::"d", requestId: "r0")
```

## Commands

```
dogwood explain policy-path.dw --policy-schema schema.cedarschema
# expected and dogwood 4ec5e2c0 and 1.1.0 (same; this text is correct for this policy):
#   Rule 1
#   PERMIT any principal to perform any action on any resource
#     when: the request context has input.user
```

```
dogwood explain policy-dotted.dw --policy-schema schema.cedarschema
# expected (the one attribute name kept apart from a path, e.g. quoted):
#   Rule 1
#   PERMIT any principal to perform any action on any resource
#     when: the request context has "input.user"
# dogwood 4ec5e2c0 and 1.1.0:
#   Rule 1
#   PERMIT any principal to perform any action on any resource
#     when: the request context has input.user
```

On 4ec5e2c0 and 1.1.0 the two outputs are byte-identical. The "expected" text for policy-dotted.dw is what 4ec5e2c0 prints with the change described under "Suspected code location". The exact wording is up to the maintainers. The point is only that the two texts must differ.

## Verdicts per decision point

```
dogwood replay policy-path.dw --policy-schema schema.cedarschema --trace trace.log
# dogwood 4ec5e2c0 and 1.1.0: @0 (time point 0): ALLOW  [rules: 0]
dogwood replay policy-dotted.dw --policy-schema schema.cedarschema --trace trace.log
# dogwood 4ec5e2c0 and 1.1.0: @0 (time point 0): DENY
```

Both verdicts are correct. The context is `{input: {user: "alice"}}`. It has the path `input.user`. It has no top-level attribute named `input.user`. Cedar 4.11.0 agrees (`cedar authorize`, same conditions, context `{"input":{"user":"alice"}}`, no entities):

```
# p-path.cedar:   permit (principal, action, resource) when { context has input.user };
# p-dotted.cedar: permit (principal, action, resource) when { context has "input.user" };
# e.json: []   ctx.json: {"input":{"user":"alice"}}
cedar authorize --policies p-path.cedar --entities e.json --principal 'User::"u"' --action 'Action::"Read"' --resource 'Doc::"d"' --context ctx.json
# cedar-policy-cli 4.11.0: ALLOW
cedar authorize --policies p-dotted.cedar --entities e.json --principal 'User::"u"' --action 'Action::"Read"' --resource 'Doc::"d"' --context ctx.json
# cedar-policy-cli 4.11.0: DENY
```

The bug is in the prose only. Someone who reads the explanation of policy-dotted.dw would expect ALLOW at @0, but the policy gives DENY. Someone who reads the explanation of policy-path.dw cannot tell it apart from policy-dotted.dw.

## Spec text

`dogwood-language/src/informalize.rs`, module doc, "# Contract" (lines 7-8):

> The output is **meaning-preserving**: read literally, the prose states the same condition as the policy.

`ParsedPolicySet::informalize` (`dogwood-language/src/policy_set.rs` line 278) points to this module: "See the `informalize` module for the translation contract."

The other outputs of the CLI keep the two apart (run on 4ec5e2c0 and 1.1.0; for `lower`, only the Cedar policy line is shown, not the header and augmented schema around it):

```
dogwood format policy-dotted.dw
# dogwood 4ec5e2c0 and 1.1.0: permit (principal, action, resource)
#                             when { context has "input.user" };
dogwood lower policy-path.dw --policy-schema schema.cedarschema
# dogwood 4ec5e2c0 and 1.1.0: permit(principal, action, resource) when { (context has input) && ((context.input) has user) };
dogwood lower policy-dotted.dw --policy-schema schema.cedarschema
# dogwood 4ec5e2c0 and 1.1.0: permit(principal, action, resource) when { context has "input.user" };
```

## Engines

The default, slice_leaves and partitioned engines were not run one by one (`dogwood replay` has no engine option). The policies have no temporal part, and the bug is in `informalize`, which plays no part in authorization. The verdicts above come from the CLI `replay` and match Cedar 4.11.0.

## Variants

All were run on 4ec5e2c0 and 1.1.0 with `dogwood explain`. Each condition is used as `permit (principal, action, resource) when { <cond> };`.

Fail (two different conditions, identical text):

```
context has input.user           => when: the request context has input.user
context has "input.user"         => when: the request context has input.user
principal has a.b                => when: the principal has a.b
principal has "a.b"              => when: the principal has a.b
```

The verdicts were only run for the `context` pair (above). For the `principal` pair, only the prose was compared.

No problem (same meaning and same text, or different texts):

```
context.input has user           => when: the request's input has user
context.input has "user"         => when: the request's input has user
context.input.user == "x"        => when: the request's user == "x"
context["input.user"] == "x"     => when: the request's input.user == "x"
```

`context.input has user` and `context.input has "user"` mean the same thing, so the same text is right. Attribute access (`.` / `["..."]`) gave different texts for this schema. The fix was not checked on other schemas.

With the change described below applied to 4ec5e2c0, both failing pairs give different texts (the dotted name is printed in quotes) and the other lines do not change.

## Suspected code location

`dogwood-language/src/informalize.rs`, function `cedar_expr` (line 1000), the `ExprKind::HasAttr` arm (line 1079):

```
        ExprKind::HasAttr { expr, attrs } => {
            let inner = cedar_operand(expr, facts);
            Phrase::inline(
                format!("{{0}} has {}", esc(&attrs.join("."))),
```

`attrs.join(".")` gives the same string for `["input", "user"]` and `["input.user"]`. Quoting a segment that contains `.` or `"` before the join made both failing pairs pass. Other non-identifier names were left as they are, because the upstream test `braces_in_author_text_are_literal` expects `has "{0}"` to read `has {0}`. With that change, `cargo test -p dogwood-language -p dogwood-docs` passes.

## Can it flip a Deny to an Allow?

Not in the decision. Verdicts come from the policy, not from the prose, and they match Cedar. The risk is to a person who reviews or approves a policy by reading `dogwood explain`. With the `permit` above, the prose looks like the path check, so the reader expects ALLOW when `context.input.user` is set, but the policy denies. With a `forbid` written the same way, it goes the other way. `permit (principal, action, resource); forbid (principal, action, resource) when { context has "input.user" };` explains its second rule as "FORBID any principal to perform any action on any resource / when: the request context has input.user", and the request above, which carries `input.user`, is allowed (`@0 (time point 0): ALLOW  [rules: 0]`). With this schema, `dogwood validate` does warn that the dotted policy "is impossible" (``warning: for policy `policy_1`, policy is impossible: the policy expression evaluates to false for all valid requests``), so a reviewer who also runs `validate` gets a hint; one who only reads `explain` does not.

Filed as a GitHub issue at the Dogwood maintainers' request.

Found by an automated property-based tester (ephyra); this report was drafted with AI assistance, and every command above was re-run on trunk 4ec5e2c0 and crates.io 1.1.0.
