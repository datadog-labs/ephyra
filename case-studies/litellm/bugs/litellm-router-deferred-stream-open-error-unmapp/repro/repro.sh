#!/usr/bin/env bash
# usage: repro.sh <python with litellm importable>   exit 1 = bug reproduces, 0 = fixed, 2 = cannot decide
PY=${1:-python3}
cd "$(dirname "$0")" || exit 2
"$PY" -c "import litellm" 2>/dev/null || { echo "litellm not importable with $PY"; exit 2; }
out=$(LITELLM_LOCAL_MODEL_COST_MAP=True NO_PROXY='*' timeout 120 "$PY" poc.py 2>/dev/null); rc=$?
printf '%s\n' "$out"
[ $rc -ne 0 ] && { echo "poc failed (rc=$rc)"; exit 2; }
grep -q "^acompletion: litellm.exceptions BadRequestError" <<<"$out" || { echo "control path not mapped: cannot decide"; exit 2; }
grep -q "^isinstance(e, litellm.BadRequestError): False" <<<"$out" && { echo "BUG: Router raised the raw provider error"; exit 1; }
grep -q "^isinstance(e, litellm.BadRequestError): True" <<<"$out" && { echo "fixed: Router raised BadRequestError"; exit 0; }
exit 2
