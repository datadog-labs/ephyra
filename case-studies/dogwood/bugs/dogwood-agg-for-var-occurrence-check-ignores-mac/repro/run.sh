#!/usr/bin/env bash
# usage: run.sh <path-to-dogwood-binary>
# Exit 1 = bug present, 0 = not present, 2 = cannot decide.
cd "$(dirname "$0")" || exit 2
D="${1:?usage: run.sh <dogwood-binary>}"
[ -x "$D" ] || exit 2
echo "== dogwood validate policy-inlined.dw (control: the macro call written out by hand)"
"$D" validate policy-inlined.dw --policy-schema schema.cedarschema || { echo "-> control rejected: cannot decide"; exit 2; }
echo "== dogwood validate policy.dw (the same policy through a macro)"
out=$("$D" validate policy.dw --policy-schema schema.cedarschema 2>&1); rc=$?
echo "$out"
echo "== dogwood replay policy.dw --trace trace.log"
"$D" replay policy.dw --policy-schema schema.cedarschema --trace trace.log
case $rc in
  0) exit 0 ;;
  2) grep -q "does not occur in the aggregation body" <<<"$out" && exit 1; exit 2 ;;
  *) exit 2 ;;
esac
