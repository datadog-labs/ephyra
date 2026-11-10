#!/usr/bin/env bash
# usage: repro.sh <python with litellm importable>   exit 1 = bug reproduces, 0 = fixed, 2 = cannot decide
PY=${1:-python3}
cd "$(dirname "$0")" || exit 2
"$PY" -c "import litellm" 2>/dev/null || { echo "litellm not importable with $PY"; exit 2; }
out=$(LITELLM_LOCAL_MODEL_COST_MAP=True NO_PROXY='*' timeout 120 "$PY" poc.py 2>/dev/null); rc=$?
printf '%s\n' "$out"
[ $rc -ne 0 ] && { echo "poc failed (rc=$rc)"; exit 2; }
grep -q "^aresponses: requests=2 status=completed" <<<"$out" || { echo "async control did not retry: cannot decide"; exit 2; }
grep -q "^responses: requests=1 raised" <<<"$out" && { echo "BUG: sync Router.responses() made one request and raised"; exit 1; }
grep -q "^responses: requests=2 status=completed" <<<"$out" && { echo "fixed: sync Router.responses() retried"; exit 0; }
exit 2
