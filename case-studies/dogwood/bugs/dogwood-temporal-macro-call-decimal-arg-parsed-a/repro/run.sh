#!/usr/bin/env bash
# usage: run.sh <path-to-dogwood-binary>
# Exit 1 = bug present, 0 = not present, 2 = cannot decide.
cd "$(dirname "$0")" || exit 2
D="${1:?usage: run.sh <dogwood-binary>}"
[ -x "$D" ] || exit 2
run() { # <policy>
  echo "== dogwood replay $1 --policy-schema schema.cedarschema --trace trace.log"
  out=$("$D" replay "$1" --policy-schema schema.cedarschema --trace trace.log 2>&1); rc=$?
  echo "$out"; echo "(exit $rc)"
}
run policy-inlined.dw   # macro body inlined, decimal("1.50") in place: DENY
run policy-control.dw   # macro called with context.input.amt: DENY
run policy.dw           # macro called with decimal("1.50"): should also DENY
case "$rc" in
  0) echo "$out" | grep -q 'DENY' && exit 0; exit 2 ;;
  *) echo "$out" | grep -q 'unknown macro `decimal`' && exit 1; exit 2 ;;
esac
