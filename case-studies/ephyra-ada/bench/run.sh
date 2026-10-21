#!/bin/bash
# Bug benchmark: for each variant in bench/mutants/ and each seed, builds the
# harness against it (EA_VENDOR, own CARGO_TARGET_DIR), runs all properties at
# TESTS cases, and appends one row per property to results/<stamp>.tsv:
#   variant  seed  property  result(pass|fail)  passing_cases_before_failure
# Runs are hermetic: EPHYRA_REGRESSIONS=0 turns off ephyra's saved
# counterexamples, which would otherwise make any bug found once "detected"
# instantly in every later run. A variant counts as detected for a seed when
# any property fails. The
# control must never fail. Split (fixed before any tuning; see SPLIT):
#   tuning:   m1278 m1280 m1282 m1285 m1326
#   held-out: m1281 m1284 midna97 (a monitor: never tune against these)
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$HERE/..
TESTS=${TESTS:-30000}
SEEDS=${SEEDS:-"11 22 33"}
# FILTER: test names to run (all when empty), for measuring new properties
# alone; unchanged properties keep their earlier rows.
FILTER=${FILTER:-}
VARIANTS=${VARIANTS:-"control m1278 m1280 m1282 m1285 m1326 m1281 m1284 midna97"}
mkdir -p "$HERE/results" "$HERE/logs"
OUT=$HERE/results/$(date +%Y%m%d-%H%M%S)-${LABEL:-run}.tsv
printf 'variant\tseed\tproperty\tresult\tpassing_before_failure\n' > "$OUT"
cd "$ROOT"
for v in $VARIANTS; do
  export EA_VENDOR=$HERE/mutants/$v
  export CARGO_TARGET_DIR=$ROOT/target/bench/$v
  cargo build -q --tests >/dev/null 2>&1
  for seed in $SEEDS; do
    log=$HERE/logs/${LABEL:-run}-$v-$seed.log
    EPHYRA_REGRESSIONS=0 EPHYRA_SEED=$seed EPHYRA_TESTS=$TESTS EPHYRA_MAX_SIZE=250 cargo test --test oracles -- $FILTER > "$log" 2>&1 || true
    python3 - "$log" "$v" "$seed" >> "$OUT" <<'PY'
import re, sys
log, v, seed = sys.argv[1:4]
text = open(log, errors="replace").read()
results = dict(re.findall(r"^test (\w+) \.\.\. (ok|FAILED)$", text, re.M))
# Each failure block: "---- <name> stdout ----" ... "after N passing tests"
after = {}
for name, block in re.findall(r"---- (\w+) stdout ----\n(.*?)(?=\n---- |\nfailures:\n|\Z)", text, re.S):
    m = re.search(r"after (\d+) passing tests", block)
    after[name] = m.group(1) if m else "?"
for name, r in sorted(results.items()):
    print(f"{v}\t{seed}\t{name}\t{'pass' if r == 'ok' else 'fail'}\t{after.get(name, '') if r != 'ok' else ''}")
PY
    n=$(awk -F'\t' -v v="$v" -v s="$seed" '$1==v && $2==s && $4=="fail"' "$OUT" | wc -l | tr -d ' ')
    echo "$v seed $seed: $n failing properties" >&2
  done
done
echo "$OUT"
