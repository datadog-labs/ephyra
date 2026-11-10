# Posted: https://github.com/dogwood-policy/dogwood/issues/24

Filed by alpaylan, 2026-11-02. Text as posted:

**`ParsedPolicy::expanded_source` renders `-(-3)` as `--3`, which re-parses to the literal `3`**

Symptom: `expanded_source()` prints a negation over an integer literal without parentheses (`-(-3)` becomes `--3`, `-(5)` becomes `-5`). The parser folds a dash run in front of a number into the literal, so the rendered text lowers to a different Cedar policy than the original.

Version: Dogwood main 4ec5e2c01b44ff3882a231ae5d315b330b38fa25 (tag `v1.1`), still the tip of the default branch on 2026-11-02, and crates.io `dogwood-language` 1.1.0.
The same render line is in 9725a8b5 (sync of 2026-09-11), which tags v1.0 and v1.1 both contain.

This was reported to the maintainer by email on 2026-10-28; #25 (`!(-3)` rendered as `!-3`) was sent as its addendum and has the same root cause.

## Minimal input

`policy.dw`:

```
permit (principal, action, resource)
when { -(-3) == 3 };
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

## Command

The CLI has no command that prints `expanded_source`, so the repro is a small Rust program using the public API. It parses each policy, takes `expanded_source()` of the one policy, lowers both texts with `LoweredPolicySet::from_str(..).as_cedar()`, and prints both lowerings. The output below is the same with crates.io 1.1.0 and with a path dependency on main 4ec5e2c0.

`Cargo.toml`:

```toml
[package]
name = "neg-literal"
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
    let lower = |s: &str| LoweredPolicySet::from_str(s, &svc, &schema).unwrap().as_cedar().to_string();
    for src in [
        "permit (principal, action, resource) when { -(-3) == 3 };",
        "permit (principal, action, resource) when { -(5) == -5 };",
    ] {
        let parsed = ParsedPolicySet::parse(src, &svc).unwrap();
        let rendered = parsed.policies().next().unwrap().expanded_source();
        println!("source:          {src}");
        println!("expanded_source: {}", rendered.trim());
        println!("source lowers:   {}", lower(src).trim());
        println!("rendered lowers: {}", lower(&rendered).trim());
        println!();
    }
}
```

```bash
cargo run -q --release
# source:          permit (principal, action, resource) when { -(-3) == 3 };
# expanded_source: permit (principal, action, resource)
# when { ((--3) == 3) };
# source lowers:   permit(principal, action, resource) when { (-((-3))) == 3 };
# rendered lowers: permit(principal, action, resource) when { 3 == 3 };
#
# source:          permit (principal, action, resource) when { -(5) == -5 };
# expanded_source: permit (principal, action, resource)
# when { ((-5) == -5) };
# source lowers:   permit(principal, action, resource) when { (-(5)) == (-5) };
# rendered lowers: permit(principal, action, resource) when { (-5) == (-5) };
#
# expected: "rendered lowers" equal to "source lowers" in both cases
```

Expected for the first case (rendered text lowers to the same Cedar as the original):

```
source lowers:   permit(principal, action, resource) when { (-((-3))) == 3 };
rendered lowers: permit(principal, action, resource) when { (-((-3))) == 3 };
```

The same difference is visible with the CLI on the rendered text, `rendered.dw` (the comments show the Cedar policy line of `dogwood lower`; the `// ─── Cedar policies ───` header before it and the augmented schema after it are left out):

```
permit (principal, action, resource)
when { ((--3) == 3) };
```

```
dogwood lower policy.dw --policy-schema schema.cedarschema
# expected: permit(principal, action, resource) when { (-((-3))) == 3 };
# dogwood 4ec5e2c0 and 1.1.0: permit(principal, action, resource) when { (-((-3))) == 3 };
dogwood lower rendered.dw --policy-schema schema.cedarschema
# expected (same as above): permit(principal, action, resource) when { (-((-3))) == 3 };
# dogwood 4ec5e2c0 and 1.1.0: permit(principal, action, resource) when { 3 == 3 };
```

`expanded_source()` is also not idempotent here. Rendering `rendered.dw` again gives `permit (principal, action, resource)` / `when { (3 == 3) };`, not the same text.

## Verdicts per decision point

```
dogwood replay policy.dw --policy-schema schema.cedarschema --trace trace.log
# dogwood 4ec5e2c0 and 1.1.0: @0 (time point 0): ALLOW  [rules: 0]
dogwood replay rendered.dw --policy-schema schema.cedarschema --trace trace.log
# dogwood 4ec5e2c0 and 1.1.0: @0 (time point 0): ALLOW  [rules: 0]
```

Observed and expected verdict at @0: ALLOW for both texts. The verdicts match. The bug is the changed lowering, not the verdict.

## Spec text

`dogwood-language/src/policy_set.rs`, doc comment of `ParsedPolicy::expanded_source` (lines 342-346):

> Formatting and comments are not preserved; the semantics are — re-parsing and lowering the result yields the same policy this handle describes.

`dogwood-language/src/render.rs`, module doc (lines 6-8 and 14-18):

> The contract is semantic, not textual — re-parsing and lowering the output yields the same policy

> Every compound operand is wrapped in `( … )` rather than tracking Cedar's precedence/associativity, so a precedence bug is structurally impossible. Parens are erased by re-parsing, so this does not change the lowered AST, and re-rendering re-adds them identically (idempotence).

The parser comment in `dogwood-language/src/parser/mod.rs`, `build_unary` (lines 1312-1314), shows why the parentheses matter here:

> Only a *directly* negated number token folds (`-5` ⇒ `Long(-5)`); a parenthesized `-(5)` keeps a `Neg` over the positive literal, matching Cedar's own constant-folding

## Engines

The default, slice_leaves and partitioned engines were not run. The renderer is not on the authorization path: `render::render_policy` is called only from `ParsedPolicy::expanded_source`, and `informalize.rs` imports only its naming helpers. Each engine decides on the lowered set it is given. The `replay` runs above use the CLI, which uses the default engine (`Authorizer::new`).

## Variants

Each was run on 4ec5e2c0 and 1.1.0 with the program above, with the policy `permit (principal, action, resource) when { <expr> };`.

Fail (the rendered text lowers differently):

```
-(-3) == 3                                 rendered --3,  lowers to 3
-(5) == -5                                 rendered -5,   lowers to (-5) instead of -(5)
-(0) == 0                                  rendered -0,   lowers to 0 instead of -(0)
-(-(-3)) == -3                             rendered -(--3), lowers to -(3)
-(-9223372036854775807) == 9223372036854775807   rendered --9223372036854775807, lowers to the literal
```

Pass (same lowering):

```
-(1 + 2) == -3                 (operand is not a literal, it stays parenthesized)
- -3 == 3                      (already folded to 3 by the parser, renders 3)
-3 == -3                       (plain negative literal)
!(true) == false               (Not, not Neg)
```

## Suspected code location

`dogwood-language/src/render.rs`, function `render_unary` (line 242; the `Neg` arm, line 246):

```
UnOp::Neg => format!("-{}", atom(expr)),
```

`atom` (line 223) does not parenthesize a literal, and a negative `Long` literal prints as `-3`. So `Neg(Long(-3))` becomes `--3` and `Neg(Long(5))` becomes `-5`. On re-parse, `build_unary` folds each `-` of a dash run in front of a bare number into the literal.

The same unparenthesized literal also breaks `!(-3)` (rendered `!-3`, which does not parse) and `(-3).x` (rendered `-3.x`); see #25. A fix for both needs two parts: wrapping any literal under `Neg` fixes this report but not #25, and wrapping negative literals in `needs_parens` fixes #25 and `-(-3)` but not `-(5)` or `-(0)`. We tried both on a local copy; with both changes, every case in this report and in #25 round-trips.

## Can it flip a Deny to an Allow?

Not in any case tried. The rendered text evaluates to the same value as the original: `-(-N)` equals `N`, and `-(N)` equals `-N`. The bug only matters to a caller that stores or compares the `expanded_source()` text, or relies on its documented same-lowering and idempotence properties.

Filed as a GitHub issue at the Dogwood maintainers' request.

Found by an automated property-based tester (ephyra); this report was drafted with AI assistance, and every command above was re-run on trunk 4ec5e2c0 and crates.io 1.1.0.
