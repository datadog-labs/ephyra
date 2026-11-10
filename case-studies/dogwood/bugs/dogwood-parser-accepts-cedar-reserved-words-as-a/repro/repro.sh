#!/usr/bin/env bash
# Usage: repro.sh <path-to-dogwood-cli> [path-to-cedar-cli]
# Exit 1 if the bug reproduces (Dogwood accepts the reserved word `is` as a
# record key and attribute name), 0 if not.
cd "$(dirname "$0")" || exit 2
DW="${1:?usage: repro.sh <dogwood> [cedar]}"
CEDAR="${2:-}"
echo "== dogwood check-parse policy.dw"
"$DW" check-parse policy.dw
echo "== dogwood replay"
out=$("$DW" replay policy.dw --policy-schema schema.cedarschema --trace trace.log 2>&1)
echo "$out"
if [ -n "$CEDAR" ]; then
  echo "== cedar check-parse policy.cedar"
  "$CEDAR" check-parse --policies policy.cedar
fi
# Fixed behaviour: replay fails with "this identifier is reserved and cannot be used: is".
grep -q 'ALLOW' <<<"$out" && exit 1
exit 0
