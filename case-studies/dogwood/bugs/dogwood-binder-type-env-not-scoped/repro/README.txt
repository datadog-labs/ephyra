run.sh <dogwood-binary>   CLI repro: three `dogwood validate` runs plus one replay.
                          Exit 1 = bug present, 0 = not present, 2 = cannot decide.
schema.cedarschema        Action schema: Login, context input = { user: String, n: Long }.
policy.dw                 Mistyped use (`input.user: u` with u: Long) hidden by a sibling
                          `exists (u: String)`. Spec: rejected. Dogwood: accepted.
policy-renamed.dw         The same policy with the second binder renamed to `w`. Rejected (control).
policy-intended.dw        Both binders declared String (what the author presumably meant). Accepted.
policy-spurious.dw        Well-typed policy (u: Long used as input.n, u: String as input.user).
                          Spec: accepted. Dogwood: rejected.
trace.log                 One Login request, for `dogwood replay`.
engines/                  Rust program: Validator result plus verdicts from the global,
                          slice_leaves and partitioned engines. Edit DOGWOOD_SRC in
                          engines/Cargo.toml to point at an upstream checkout, then
                          cargo run -- ../policy.dw ../schema.cedarschema ../trace.log
