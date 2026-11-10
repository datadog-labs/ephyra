# Posted: https://github.com/dogwood-policy/dogwood/issues/27

Filed by alpaylan, 2026-11-02. Text as posted:

**`dogwood explain` does not parenthesize an `if` that is an operand of `&&`/`||`, so two policies with different verdicts get identical prose**

Symptom: when an `if … then … else …` is an operand of `||` or `&&`, `dogwood explain` (`ParsedPolicySet::informalize`) prints it without parentheses. Its trailing ", otherwise …" branch then also reads as covering the next "or …"/"and …" row. `((if c then false else false) || true) && true` and `(if c then false else (false || true)) && true` get byte-identical explanations. At `n = 0` the first allows and the second denies.

Version: Dogwood main 4ec5e2c01b44ff3882a231ae5d315b330b38fa25 (tag `v1.1`), still the tip of the default branch on 2026-11-02, and crates.io `dogwood-cli` 1.1.0 (same output).

The same happens when the `&&`/`||` is wrapped in `!( … )`; a second example is below. One code change fixes both.

## Minimal input

`a.dw`:

```
permit (principal, action, resource)
when { ((if context.input.n == 0 then false else false) || true) && true };
```

`b.dw` (the same, but the `|| true` moved inside the else-branch):

```
permit (principal, action, resource)
when { (if context.input.n == 0 then false else (false || true)) && true };
```

`schema.cedarschema` (action schema):

```
entity User;
entity Doc;
action "Read" appliesTo {
  principal: [User], resource: [Doc],
  context: { input: { n: Long } }
};
```

Event schema: the default (no `--event-schema`).

`trace.log`:

```
@0 scope(principal: User::"u", resource: Doc::"d") request_context(input: { n: 0 }) Action::"Read"::request(input: { n: 0 }, callerPrincipal: User::"u", callerResource: Doc::"d", requestId: "r0")
@1 scope(principal: User::"u", resource: Doc::"d") request_context(input: { n: 1 }) Action::"Read"::request(input: { n: 1 }, callerPrincipal: User::"u", callerResource: Doc::"d", requestId: "r1")
```

## Commands and output (trunk 4ec5e2c0 and 1.1.0)

```
dogwood explain a.dw --policy-schema schema.cedarschema
# dogwood:
Rule 1
PERMIT any principal to perform any action on any resource
  when:
    if the request's n == 0 then false, otherwise false
      or true
    and true
```

```
dogwood explain b.dw --policy-schema schema.cedarschema
# dogwood (byte-identical to a.dw):
Rule 1
PERMIT any principal to perform any action on any resource
  when:
    if the request's n == 0 then false, otherwise false
      or true
    and true
```

```
dogwood replay a.dw --policy-schema schema.cedarschema --trace trace.log
# dogwood:
@0 (time point 0): ALLOW  [rules: 0]
@1 (time point 1): ALLOW  [rules: 0]
# cedar 4.11.0 (same condition as a Cedar policy, context {"input":{"n":0}}): ALLOW
```

```
dogwood replay b.dw --policy-schema schema.cedarschema --trace trace.log
# dogwood:
@0 (time point 0): DENY
@1 (time point 1): ALLOW  [rules: 0]
# cedar 4.11.0 (same condition as a Cedar policy, context {"input":{"n":0}}): DENY
```

The Cedar lines come from `cedar authorize --policies a.cedar --entities e.json --principal 'User::"u"' --action 'Action::"Read"' --resource 'Doc::"d"' --context c0.json` (cedar-policy-cli 4.11.0), where `a.cedar` is the text of `a.dw`, `e.json` is `[]` and `c0.json` is `{"input":{"n":0}}`.

Verdicts per decision point. These are correct, and only the prose is wrong:

```
@0 (n = 0): a.dw ALLOW, b.dw DENY
@1 (n = 1): a.dw ALLOW, b.dw ALLOW
```

Expected: the two explanations differ, so that read literally each states its own policy's condition. For a.dw, the `if` should be fenced, e.g. `(if the request's n == 0 then false, otherwise false)` followed by the `or true` row. A build of trunk with the fence applied to `if` operands of `&&`/`||` (see "Suspected code location") prints exactly that for a.dw. For b.dw it prints `(if the request's n == 0 then false, otherwise false` / `or true)` followed by `and true`.

## Second example: the same operand under `!( … )`

`policy-grouped.dw`:

```
permit (principal, action, resource)
when { !((if context.input.n == 0 then true else false) && false) };
```

`policy-regrouped.dw` (same parts, but the `&&` is inside the else-branch):

```
permit (principal, action, resource)
when { !(if context.input.n == 0 then true else (false && false)) };
```

Same schema and trace as above.

```
dogwood explain policy-grouped.dw --policy-schema schema.cedarschema
dogwood explain policy-regrouped.dw --policy-schema schema.cedarschema
# dogwood 4ec5e2c0 and 1.1.0, both (byte-identical):
#   Rule 1
#   PERMIT any principal to perform any action on any resource
#     when:
#       not (if the request's n == 0 then true, otherwise false
#         and false)
# with the fix, policy-grouped.dw:
#       not ((if the request's n == 0 then true, otherwise false)
#         and false)
dogwood replay policy-grouped.dw --policy-schema schema.cedarschema --trace trace.log
# @0 (time point 0): ALLOW  [rules: 0]
# @1 (time point 1): ALLOW  [rules: 0]
dogwood replay policy-regrouped.dw --policy-schema schema.cedarschema --trace trace.log
# @0 (time point 0): DENY
# @1 (time point 1): ALLOW  [rules: 0]
# cedar 4.11.0 (context {"input":{"n":0}} then {"input":{"n":1}}): grouped ALLOW, ALLOW; regrouped DENY, ALLOW
```

At @0, n == 0: grouped is `!(true && false)`, which is ALLOW. Regrouped is `!true`, which is DENY. At @1 both are `!(false && false)`, which is ALLOW.

`--format json` keeps the tree structure (the `if` is a child of the `{0} and {1}` node), so a JSON consumer can still see the grouping. The human text cannot show it.

## Spec text

`dogwood-docs/guide/12-cli.md`, section "`explain` — read a policy set back in semi-English":

> The translation is deterministic — a pure function of the parsed, macro-expanded policy plus the schemas, with no language model — and meaning-preserving: read literally, it states the same condition as the policy.

`dogwood-language/src/informalize.rs`, module doc, "# Contract" (lines 7-8):

> The output is **meaning-preserving**: read literally, the prose states the same condition as the policy.

Same module doc (lines 40-43):

> The only synthesized phrases are framing: the rule header, the event schema's pins stated once as a preamble, and the `(…)` that fences a compound operand so the prose keeps the AST's grouping (as [`crate::render`] fully parenthesizes).

Comment in the `IfThenElse` arm of `cedar_expr` (lines 1121-1126):

> The else-branch runs to the end of the phrase, so an `else if` chain reads naturally and needs no parentheses; the whole `if` is fenced wherever it is itself an operand (see `cedar_operand`).

`ParsedPolicySet::informalize` (`dogwood-language/src/policy_set.rs` line 278) refers to this module in its doc comment (line 277): "See the `informalize` module for the translation contract."

Two policies with different verdicts cannot both have the same literal reading.

## Engines

The bug is in the explanation, not in evaluation. The policies have no temporal part, and `informalize` takes no part in authorization. `dogwood replay` uses the default engine; the slice_leaves and partitioned engines were not run one by one. The verdicts above match Cedar 4.11.0.

## Variants

All were run on 4ec5e2c0 and 1.1.0 with `dogwood explain` and `dogwood replay`. Each side is used as `permit (principal, action, resource) when { <expr> };`, with C = `context.input.n == 0`. Verdicts are at @0 (n = 0), left/right.

Fail (byte-identical prose for the two groupings, different verdicts):

```
((if C then false else false) || true) && true    vs  (if C then false else (false || true)) && true    ALLOW/DENY  (a.dw / b.dw)
((if C then true else false) && false) || false   vs  (if C then true else (false && false)) || false   DENY/ALLOW
true && ((if C then false else false) || true)    vs  true && (if C then false else (false || true))    ALLOW/DENY
!((if C then true else false) && false)           vs  !(if C then true else (false && false))           ALLOW/DENY  (second example)
!((if C then false else false) || true)           vs  !(if C then false else (false || true))           DENY/ALLOW
((if C then true else false) && false) == true    vs  (if C then true else (false && false)) == true    DENY/ALLOW
```

Do not fail (the prose differs):

```
(if C then true else false) && false              vs  if C then true else (false && false)
(if C then false else false) || true              vs  if C then false else (false || true)
!((true && (if C then false else true)) || true)  vs  !(true && (if C then false else (true || true)))
((if C then false else false) == true) && true    vs  (if C then false else (false == true)) && true
```

In the first three passing pairs the only difference is indentation. When the `&&` / `||` is the top of the `when` clause, its second row is less indented than an `else` row would be. When the `if` is the right operand, its rows are indented one level deeper. There are no parentheses in any of them. In the last pair the `if` is an operand of `==`, which is fenced through `cedar_operand`.

With the change described below applied to 4ec5e2c0, every pair above gives different texts. The `if` is printed as `(if ... , otherwise ...)`.

## Suspected code location

`dogwood-language/src/informalize.rs`, function `cedar_expr` (line 1000), the `BinOp::And | BinOp::Or` arm (line 1037):

```
            // `&&` / `||` operands sit on their own rows, so the layout
            // already shows the grouping.
            BinOp::And | BinOp::Or => {
                let l = cedar_expr(left, facts);
                let r = cedar_expr(right, facts);
```

The operands go through `cedar_expr`, not `cedar_operand`, so an `if` operand is never fenced. The `IfThenElse` arm's comment says it relies on the `if` being fenced wherever it is an operand. Rendering an `IfThenElse` operand of `&&` / `||` with `cedar_operand` (other operands unchanged) made every failing pair above pass and printed the "with the fix" texts shown above.

## Can it flip a Deny to an Allow?

Not in the decision. Verdicts are computed from the policy, not from the prose, and they match Cedar. The risk is to a person who reviews or approves a policy by reading `dogwood explain`. In the second example they would read the policy-regrouped.dw meaning (DENY when n == 0), but the deployed policy-grouped.dw allows that request. So a reviewer can be misled into accepting a policy that allows requests they think it denies. The opposite also happens: in the first example b.dw denies the `n = 0` request, and its prose cannot be told apart from a.dw, which allows it.

Filed as a GitHub issue at the Dogwood maintainers' request.

Found by an automated property-based tester (ephyra); this report was drafted with AI assistance, and every command above was re-run on trunk 4ec5e2c0 and crates.io 1.1.0.
