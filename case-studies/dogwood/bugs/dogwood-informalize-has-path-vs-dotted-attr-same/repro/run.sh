#!/usr/bin/env bash
# Usage: run.sh <path-to-dogwood-binary>
# Runs `dogwood explain` and `dogwood replay` on two policies:
#   policy-path.dw:   context has input.user     (path: context.input.user)
#   policy-dotted.dw: context has "input.user"   (one attribute named "input.user")
# Exit 1: the two explanations are identical (bug reproduces). 0: they differ.
# Other: could not run.
set -u
here="$(cd "$(dirname "$0")" && pwd)"
D="${1:?usage: run.sh <path-to-dogwood-binary>}"
[ -x "$D" ] || { echo "not executable: $D" >&2; exit 2; }
S="$here/schema.cedarschema"
for f in path dotted; do
  echo "== policy-$f.dw"; cat "$here/policy-$f.dw"
  echo "-- dogwood explain"
  "$D" explain "$here/policy-$f.dw" --policy-schema "$S" || exit 2
  echo "-- dogwood replay"
  "$D" replay "$here/policy-$f.dw" --policy-schema "$S" --trace "$here/trace.log" || exit 2
  echo
done
a=$("$D" explain "$here/policy-path.dw" --policy-schema "$S") || exit 2
b=$("$D" explain "$here/policy-dotted.dw" --policy-schema "$S") || exit 2
if [ "$a" = "$b" ]; then
  echo "RESULT: identical explanations for policies that decide differently (bug reproduces)"
  exit 1
fi
echo "RESULT: explanations differ (bug does not reproduce)"
exit 0
