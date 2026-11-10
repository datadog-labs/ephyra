# Posted: https://github.com/dogwood-policy/dogwood/issues/25

Filed by alpaylan, 2026-11-02. Text as posted:

**`ParsedPolicy::expanded_source` renders `!(-3)` as `!-3`, which Dogwood's parser rejects**

Symptom: `expanded_source()` prints a `!` over a negative integer literal without parentheses (`!(-3)` becomes `!-3`). Dogwood's parser rejects `!-3` with "unexpected token `-`, expected expression", so the rendered text no longer parses. The same missing parentheses turn a member or method access on a negative literal, `(-3).x`, into `-3.x`, which re-parses as `-(3.x)`.

Version: Dogwood main 4ec5e2c01b44ff3882a231ae5d315b330b38fa25 (tag `v1.1`), still the tip of the default branch on 2026-11-02, and crates.io `dogwood-language` 1.1.0.

Addendum to #24 (`-(-3)` rendered as `--3`, reported by email on 2026-10-28), same root cause: `atom()` in `render.rs` wraps an operand only when `needs_parens()` is true, which is never the case for a literal, and a negative `Long` literal prints with a leading `-`. #24 is that literal under `-`; this report is the same literal under `!` and as a member/method receiver. The fix needs two parts: changing only the `UnOp::Neg` arm fixes #24 but not these cases, and wrapping negative literals in `needs_parens` fixes these cases and `-(-3)` but not `-(5)`. We tried both on a local copy.

## Minimal input

`policy.dw`:

```
permit (principal, action, resource)
when { !(-3) };
```

`schema.cedarschema` (action schema):

```
entity User;
entity Doc;
action "Read" appliesTo { principal: [User], resource: [Doc] };
```

Event schema: the default (`ServiceSchema::builder().build()`).

`trace.log` (only used for the `replay` check below):

```
@0 scope(principal: User::"u", resource: Doc::"d") Action::"Read"::request(callerPrincipal: User::"u", callerResource: Doc::"d", requestId: "r0")
```

The same thing happens through a macro (`macro.dw`):

```
def cedar not_(?p) { !?p };
permit (principal, action, resource)
when { not_(-3) };
```

## Command

The CLI has no command that prints `expanded_source`, so the repro is a small Rust program using the public API. It parses each policy, takes `expanded_source()` of the one policy, lowers both texts with `LoweredPolicySet::from_str(..).as_cedar()`, and prints both lowerings. It covers `policy.dw`, the macro form and the receiver case. The output below is the same with crates.io 1.1.0 and with a path dependency on main 4ec5e2c0.

`Cargo.toml`:

```toml
[package]
name = "not-neg-literal"
version = "0.1.0"
edition = "2024"

[dependencies]
dogwood-language = "1.1.0"
```

`src/main.rs`:

```rust
use dogwood_language::{LoweredPolicySet, ParsedPolicySet, PolicySchema, ServiceSchema};

const SCHEMA: &str = r#"
entity User;
entity Doc;
action "Read" appliesTo { principal: [User], resource: [Doc] };
"#;

fn main() {
    let schema = PolicySchema::from_cedarschema_str(SCHEMA).unwrap();
    let svc = ServiceSchema::defaults();
    let lower = |s: &str| match LoweredPolicySet::from_str(s, &svc, &schema) {
        Ok(set) => set.as_cedar().to_string().trim().to_string(),
        Err(e) => format!("ERROR: {e}"),
    };
    for src in [
        "permit (principal, action, resource) when { !(-3) };",
        "def cedar not_(?p) { !?p };\npermit (principal, action, resource) when { not_(-3) };",
        "permit (principal, action, resource) when { (-3).x == 1 };",
    ] {
        let parsed = ParsedPolicySet::parse(src, &svc).unwrap();
        let rendered = parsed.policies().next().unwrap().expanded_source();
        println!("source:          {}", src.replace('\n', " "));
        println!("expanded_source: {}", rendered.trim().replace('\n', " "));
        println!("source lowers:   {}", lower(src));
        println!("rendered lowers: {}", lower(&rendered));
        println!();
    }
}
```

```bash
cargo run -q --release
# source:          permit (principal, action, resource) when { !(-3) };
# expanded_source: permit (principal, action, resource) when { !-3 };
# source lowers:   permit(principal, action, resource) when { !(-3) };
# rendered lowers: ERROR: unexpected token `-`, expected expression
#
# source:          def cedar not_(?p) { !?p }; permit (principal, action, resource) when { not_(-3) };
# expanded_source: permit (principal, action, resource) when { !-3 };
# source lowers:   permit(principal, action, resource) when { !(-3) };
# rendered lowers: ERROR: unexpected token `-`, expected expression
#
# source:          permit (principal, action, resource) when { (-3).x == 1 };
# expanded_source: permit (principal, action, resource) when { (-3.x == 1) };
# source lowers:   permit(principal, action, resource) when { ((-3).x) == 1 };
# rendered lowers: permit(principal, action, resource) when { (-(3.x)) == 1 };
#
# expected: "rendered lowers" equal to "source lowers" in all three cases
```

Expected for the first case:

```
expanded_source: permit (principal, action, resource) when { !(-3) };
source lowers:   permit(principal, action, resource) when { !(-3) };
rendered lowers: permit(principal, action, resource) when { !(-3) };
```

The macro form prints the same `expanded_source()` text (`when { !-3 }`) and the same error.

With the CLI, on the rendered text (`rendered.dw`: `permit (principal, action, resource)` / `when { !-3 };`):

```
dogwood replay rendered.dw --policy-schema schema.cedarschema --trace trace.log
# expected: parses (same as policy.dw)
# dogwood 4ec5e2c0 and 1.1.0 (exit 2):
#     × unexpected token `-`, expected expression
#      ╭─[2:9]
#    1 │ permit (principal, action, resource)
#    2 │ when { !-3 };
#      ·         ┬
#      ·         ╰── unexpected token `-`, expected expression
#      ╰────
```

Cedar 4.11.0 (the language Dogwood's grammar follows) also rejects the rendered form and accepts the original:

```
cedar check-parse --policies <file with: permit(principal, action, resource) when { !-3 };>
# cedar-policy-cli 4.11.0 (exit 1):
#     × failed to parse policy set
#     ╰─▶ unexpected token `-`
#      ╭────
#    1 │ permit(principal, action, resource) when { !-3 };
#      ·                                             ┬
#      ·                                             ╰── expected `!`, `(`, `[`, `{`, `false`, identifier, `if`, number, `?principal`, `?resource`, string literal, or `true`
#      ╰────
cedar check-parse --policies <file with: permit(principal, action, resource) when { !(-3) };>
# cedar-policy-cli 4.11.0: no output (exit 0)
```

## Verdicts per decision point

```
dogwood replay policy.dw --policy-schema schema.cedarschema --trace trace.log
# dogwood 4ec5e2c0 and 1.1.0: @0 (time point 0): DENY
#                               error: error while evaluating policy `policy_0`: type error: expected bool, got long
dogwood replay rendered.dw --policy-schema schema.cedarschema --trace trace.log
# dogwood 4ec5e2c0 and 1.1.0: no verdict; the file does not parse
```

Observed at @0: DENY for the original (the policy errors and is skipped). The rendered text gives no verdict because it does not parse. Expected: the rendered text parses and gives the same DENY. The bug is that the rendered text does not parse. It does not change a verdict.

## Spec text

`dogwood-language/src/policy_set.rs`, doc comment of `ParsedPolicy::expanded_source` (lines 342-346):

> This policy's source, re-rendered from its parsed, macro-expanded form: valid `.dw` text in which every macro is inlined and no macro reference remains. Formatting and comments are not preserved; the semantics are — re-parsing and lowering the result yields the same policy this handle describes.

`dogwood-language/src/render.rs`, module doc (lines 14-18):

> **Full parenthesization.** Every compound operand is wrapped in `( … )` rather than tracking Cedar's precedence/associativity, so a precedence bug is structurally impossible.

`dogwood-syntax/src/grammar.pest`, comment above `unary` (lines 327-331), on why `!-3` is not valid `.dw`:

> Upstream distinguishes homogeneous runs of `!` (logical not) from homogeneous runs of `-` (arithmetic negation) and rejects mixed combinations such as `!-x` or `-!x` [...] We encode the same here

## Engines

The default, slice_leaves and partitioned engines were not run. The renderer is not on the authorization path: `render::render_policy` is called only from `ParsedPolicy::expanded_source`, and `informalize.rs` imports only its naming helpers (`BinderRenames`, `binder_renames`, `quote`). Each engine decides on the lowered set it is given. The `replay` runs above use the CLI's default engine.

## Variants

Each was run on 4ec5e2c0 and 1.1.0 with the program above, with the policy `permit (principal, action, resource) when { <expr> };`.

Fail:

```
!(-3)                      rendered !-3,      does not parse
!!(-3)                     rendered !(!-3),   does not parse
!(-3) || true              rendered ((!-3) || true), does not parse
not_(-3) via def cedar     rendered !-3,      does not parse
(-3).isEmpty()             rendered -3.isEmpty(),   lowers to -(3.isEmpty())
(-3).contains(1)           rendered -3.contains(1), lowers to -(3.contains(1))
(-3).x == 1                rendered (-3.x == 1),    lowers to (-(3.x)) == 1
(-3)["x"] == 1             rendered (-3.x == 1),    lowers to (-(3.x)) == 1
```

Pass (same lowering):

```
!(3)              rendered !3
!(0 - 3)          rendered !(0 - 3)
(-3) == -3        rendered (-3 == -3)
(-3) has x        rendered -3 has "x"
```

`!(-(3))` and `!(-(-3))` also fail, but through the `-(...)` case of #24.

All the failing inputs are ill-typed: `!` needs a bool, and a Long has no attributes or methods. Dogwood still accepts and lowers them, so the contract above applies to them.

With the change described below applied to 4ec5e2c0, every input in the Fail list above gives the same lowering for the original and the rendered text. `!(-(3))` still fails with that change alone, because it is the `-(5)` case of #24; with both changes all of them round-trip.

## Suspected code location

`dogwood-language/src/render.rs`, function `needs_parens` (line 231), used by `atom` (line 223):

```
fn needs_parens(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::UnaryApp { op, .. } => matches!(op, UnOp::Not | UnOp::Neg),
        ExprKind::Is { .. }
        | ExprKind::Like { .. }
        | ExprKind::HasAttr { .. }
        | ExprKind::IfThenElse { .. } => true,
        _ => false,
    }
}
```

A literal is never wrapped, but a negative `Long` literal prints with a leading `-`. So `render_unary` (`UnOp::Not => format!("!{}", atom(expr))`, line 245) gives `!-3`, and a member or method call on the literal gives `-3.x`. Returning true for a negative `Long` literal in `needs_parens` made all the failing variants above pass.

## Can it flip a Deny to an Allow?

Not in any case tried. A `!` over a negative literal is always a type error at evaluation, so the policy errors and is skipped. The rendered text either does not parse (the caller gets an error, not a verdict) or, in the receiver case, lowers to `-(3.x)`, which also errors. The bug only matters to a caller that stores or re-parses the `expanded_source()` text.

Filed as a GitHub issue at the Dogwood maintainers' request.

Found by an automated property-based tester (ephyra); this report was drafted with AI assistance, and every command above was re-run on trunk 4ec5e2c0 and crates.io 1.1.0.
