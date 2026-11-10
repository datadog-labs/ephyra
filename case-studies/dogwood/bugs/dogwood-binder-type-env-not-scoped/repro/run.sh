#!/usr/bin/env bash
# usage: run.sh <path-to-dogwood-binary>
# Exit 1 = bug present, 0 = not present, 2 = cannot decide.
cd "$(dirname "$0")" || exit 2
D="${1:?usage: run.sh <dogwood-binary>}"
[ -x "$D" ] || exit 2
bug=0
check() { # <policy> <expected: ok|error>
  out=$("$D" validate "$1" --policy-schema schema.cedarschema 2>&1); rc=$?
  echo "== dogwood validate $1   (exit $rc)"; echo "$out"
  case "$2:$rc" in
    ok:0|error:2) ;;
    ok:2|error:0) echo "-> unexpected (spec: $2)"; bug=1 ;;
    *) echo "-> cannot decide"; exit 2 ;;
  esac
}
check policy.dw error          # mistyped use of the Long binder: should be rejected
check policy-renamed.dw error  # same policy, inner binder renamed: rejected (control)
check policy-spurious.dw ok    # well-typed policy: should be accepted
echo "== dogwood replay policy.dw --trace trace.log"
"$D" replay policy.dw --policy-schema schema.cedarschema --trace trace.log
exit $bug
