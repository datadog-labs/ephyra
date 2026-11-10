Run: ./repro.sh <path-to-dogwood-cli> [path-to-cedar-cli]
Exit 1: the bug reproduces (Dogwood accepts `{is: 1} has is` and replay prints ALLOW).
Exit 0: Dogwood rejects the policy.
policy.cedar is the same text, for `cedar check-parse --policies policy.cedar`.
engines-driver/main.rs: Rust driver over the public API that runs the trace on the
global, slice_leaves and partitioned engines (usage in its first line; needs a
Cargo.toml depending on dogwood-language).
