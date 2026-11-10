Build upstream Dogwood (cargo build -p dogwood-cli), then:
  ./run.sh <path-to>/target/debug/dogwood
Exit 1 = bug present (policy.dw rejected with "macro-in-macro is not supported"),
0 = not present (policy.dw replays to DENY like policy-inlined.dw), 2 = cannot decide.
