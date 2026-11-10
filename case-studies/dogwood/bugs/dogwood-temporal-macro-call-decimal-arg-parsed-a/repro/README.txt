Build Dogwood (cargo build -p dogwood-cli), then:
  ./run.sh <path-to>/target/debug/dogwood
Exit 1 = bug present (policy.dw rejected with "unknown macro `decimal`"),
exit 0 = fixed (policy.dw replays to DENY like the other two), 2 = cannot decide.
