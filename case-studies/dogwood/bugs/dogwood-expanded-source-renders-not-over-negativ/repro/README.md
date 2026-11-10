Run: `./run.sh <path-to-dogwood-checkout> [policy.dw]`

Builds a short program (poc/) against `<checkout>/dogwood-language` by path. It parses the policy,
calls `ParsedPolicy::expanded_source()`, lowers both texts and prints the Cedar for each (or the error).
Exit 1: the rendered text fails to lower, or lowers differently (bug). Exit 0: they match.
Exit 2: build or setup error. Needs cargo and network for crates. Set CARGO_TARGET_DIR to reuse a build.

`policy.dw` is the minimal case (`when { !(-3) }`). `macro.dw` is the same through a `def cedar` macro.
`rendered.dw` is the text expanded_source() returns for `policy.dw` on 4ec5e2c0.
`trace.log` is a one-event trace for `dogwood replay`. `schema.cedarschema` is the action schema.
