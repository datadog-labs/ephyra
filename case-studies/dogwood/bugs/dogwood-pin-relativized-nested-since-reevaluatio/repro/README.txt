run.sh <dogwood-binary> [N=16] [limit-seconds=30] [policy=policy.dw]
  Replays the policy over N same-principal Write requests (gen-trace.sh) with
  the unpinned schema and with the default schema; exit 1 if the default-schema
  run exceeds the limit. trace.log is the N=16 trace.
policy.dw:  nesting depth 2 (slow on trunk 4ec5e2c0).
policy3.dw: nesting depth 3 (still slow on 4ec5e2c0 plus the separate fix for
  the nested-`previous` report), e.g. run.sh <dogwood> 16 15 policy3.dw
engines/: cargo project timing the global, slice_leaves and partitioned
  engines (edit the dogwood-language path in Cargo.toml), e.g.
  engines global ../policy.dw ../schema.cedarschema ../trace.log
