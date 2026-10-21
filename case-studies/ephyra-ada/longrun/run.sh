#!/bin/bash
# Long bug-hunting run on vendor/ (ada main + open fixes): all properties,
# fresh seeds, saved counterexamples on. One log per seed and size. Runs a
# frozen copy of the test binary (longrun/oracles.bin, from `cargo test
# --release --test oracles --no-run`), so editing the crate meanwhile does
# not change what runs.
cd "$(dirname "$0")/.."
TESTS=${TESTS:-3000000}
for spec in "7001 250" "7002 1000" "7003 250" "7004 1000"; do
  set -- $spec
  log=longrun/$(date +%Y%m%d-%H%M%S)-seed$1-size$2.log
  EPHYRA_SEED=$1 EPHYRA_MAX_SIZE=$2 EPHYRA_TESTS=$TESTS longrun/oracles.bin > "$log" 2>&1
  echo "seed $1 size $2: $(grep -c '\.\.\. FAILED' "$log") failing ($log)"
done
