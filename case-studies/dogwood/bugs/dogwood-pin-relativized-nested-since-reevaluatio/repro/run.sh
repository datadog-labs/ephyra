#!/usr/bin/env bash
# Replays a nested positive-left `since` policy over N same-principal Write
# requests, 1 s apart, under the built-in unpinned event schema and under the
# default (callerPrincipal-pinned) schema, and prints the wall time of each.
# usage: run.sh <path-to-dogwood-binary> [N=16] [limit-seconds=30] [policy=policy.dw]
# exit 1: default-schema replay did not finish within the limit (bug present)
# exit 0: it finished within the limit (verdicts printed)
# exit 2: usage or setup error, or the two schemas gave different verdicts
cd "$(dirname "$0")" || exit 2
DW="${1:?usage: run.sh <dogwood-binary> [N] [limit-seconds] [policy]}"
N="${2:-16}"
LIMIT="${3:-30}"
POLICY="${4:-policy.dw}"
[ -x "$DW" ] || exit 2
T="$(mktemp -d)" || exit 2
trap 'rm -rf "$T"' EXIT
./gen-trace.sh "$N" > "$T/trace.log"

s=$(date +%s%N)
"$DW" replay "$POLICY" --policy-schema schema.cedarschema --trace "$T/trace.log" \
  --event-schema unpinned.dwschema > "$T/unpinned.out" || exit 2
echo "unpinned schema: $(( ($(date +%s%N) - s) / 1000000 )) ms"

s=$(date +%s%N)
timeout "$LIMIT" "$DW" replay "$POLICY" --policy-schema schema.cedarschema \
  --trace "$T/trace.log" > "$T/default.out"
rc=$?
ms=$(( ($(date +%s%N) - s) / 1000000 ))
if [ "$rc" = 124 ]; then
  echo "default schema:  did not finish within ${LIMIT} s"
  exit 1
fi
[ "$rc" = 0 ] || exit 2
echo "default schema:  $ms ms"
cmp -s "$T/unpinned.out" "$T/default.out" || { echo "verdicts differ"; diff "$T/unpinned.out" "$T/default.out"; exit 2; }
cat "$T/default.out"
exit 0
