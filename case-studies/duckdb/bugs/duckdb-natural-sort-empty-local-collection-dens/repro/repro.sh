#!/bin/bash
# usage: repro.sh <path-to-duckdb-cli>
# exit 1 = crash reproduced (process killed by a signal), 0 = correct result in all runs, 2 = cannot decide
DUCKDB=${1:?usage: repro.sh <path-to-duckdb-cli>}
cd "$(dirname "$0")" || exit 2
[ -x "$DUCKDB" ] || exit 2
for i in 1 2 3 4 5; do
  out=$(env -u AI_AGENT timeout 120 "$DUCKDB" -csv :memory: < repro.sql 2>&1)
  rc=$?
  if [ $rc -ge 128 ] && [ $rc -ne 124 ]; then echo "run $i: killed by signal $((rc-128))"; exit 1; fi
  [ $rc -eq 0 ] || { echo "run $i: rc=$rc: $out"; exit 2; }
  # expected: header + one row "NULL,1" (NULL may print as empty or NULL)
  echo "$out" | tail -1 | grep -qxE '(NULL)?,1' || { echo "run $i: unexpected output: $out"; exit 2; }
done
echo "no crash in 5 runs, result ok"; exit 0
