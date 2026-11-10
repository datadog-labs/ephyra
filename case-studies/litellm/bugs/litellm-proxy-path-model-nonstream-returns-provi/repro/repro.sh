#!/usr/bin/env bash
# Usage: repro.sh [python]   (a python with litellm[proxy] installed; default: python3)
# Exit 1 when the bug reproduces (non-stream "model" != "team-model"), 0 when it does not, 2 on setup error.
PY=${1:-python3}
cd "$(dirname "$0")" || exit 2
export LITELLM_LOCAL_MODEL_COST_MAP=True LITELLM_TELEMETRY=False NO_PROXY='*'
"$PY" mock_openai.py 18080 & M=$!
"$PY" -m litellm.proxy.proxy_cli --config config.yaml --port 14000 > proxy.log 2>&1 & P=$!
trap 'kill $M $P 2>/dev/null' EXIT
for i in $(seq 120); do curl -s -o /dev/null localhost:14000/health/liveliness && break; sleep 0.5; done
U=localhost:14000${ROUTE:-/engines/team-model/chat/completions}
D='{"messages":[{"role":"user","content":"hi"}]}'
echo "--- non-stream"; NS=$(curl -s $U -H 'Authorization: Bearer sk-1234567890abcdef' -H 'Content-Type: application/json' -d "$D"); echo "$NS"
echo "--- stream"; curl -s $U -H 'Authorization: Bearer sk-1234567890abcdef' -H 'Content-Type: application/json' -d "${D%\}},\"stream\":true}" | grep -o '"model":"[^"]*"' | sort | uniq -c
echo "--- control: same request on /chat/completions with model in the body"
curl -s localhost:14000/chat/completions -H 'Authorization: Bearer sk-1234567890abcdef' -H 'Content-Type: application/json' -d "${D%\}},\"model\":\"team-model\"}" | grep -o '"model":"[^"]*"'
M_NS=$(echo "$NS" | "$PY" -c 'import json,sys; print(json.load(sys.stdin)["model"])' 2>/dev/null) || exit 2
echo "non-stream model: $M_NS"
[ "$M_NS" = team-model ] && exit 0 || exit 1
