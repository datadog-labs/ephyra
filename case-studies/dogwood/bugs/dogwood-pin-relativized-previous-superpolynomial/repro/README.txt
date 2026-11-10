run.sh <dogwood-binary> [N=16] [limit-seconds=30]
  Replays policy.dw over N same-principal requests (gen-trace.sh) with the
  unpinned schema and with the default schema; exit 1 if the default-schema
  run exceeds the limit. trace.log is the N=16 trace.
engines/: cargo project timing the global, slice_leaves and partitioned
  engines (edit the dogwood-language path in Cargo.toml), e.g.
  engines global ../policy.dw ../schema.cedarschema ../trace.log
