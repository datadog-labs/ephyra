policy.dw           count over a for-variable used only as a macro call argument (rejected on trunk)
policy-inlined.dw   the same policy with the macro call written out by hand (accepted)
schema.cedarschema  action schema; default event schema, no providers
trace.log           three Login requests by one principal, input.user a, b, c
run.sh <dogwood>    exit 1 = bug present, 0 = not present, 2 = cannot decide
variants/           v1..v10 bodies (see report.md); validate with
                    dogwood validate variants/vN.dw --policy-schema variants/schema.cedarschema
engines/            Rust program: lowers through the public API and prints the verdicts of the
                    global, slice_leaves and partitioned engines.
                    cargo run -- ../policy-inlined.dw ../schema.cedarschema ../providers.json ../trace.log
