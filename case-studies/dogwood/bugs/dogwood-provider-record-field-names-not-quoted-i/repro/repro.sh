#!/usr/bin/env bash
# Usage: repro.sh <path-to-dogwood-cli>
# Exit 1 if the bug reproduces (lowering fails because the provider record
# field "a b" is written unquoted into a synthesized Cedar schema), 0 if not.
cd "$(dirname "$0")" || exit 2
DW="${1:?usage: repro.sh <dogwood>}"
echo "== dogwood schema providers providers.json"
"$DW" schema providers providers.json
echo "== dogwood replay"
out=$("$DW" replay policy.dw --policy-schema schema.cedarschema --providers providers.json --trace trace.log 2>&1)
echo "$out"
# Fixed behaviour: "@1 (time point 0): ALLOW  [rules: 0]".
grep -q 'synthesize providers type' <<<"$out" && exit 1
grep -q 'ALLOW' <<<"$out" && exit 0
exit 2
