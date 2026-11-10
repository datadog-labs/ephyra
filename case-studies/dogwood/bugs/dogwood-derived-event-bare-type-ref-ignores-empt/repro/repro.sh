#!/usr/bin/env bash
# Usage: repro.sh <path-to-dogwood-binary>
# Validates the same policy against two equivalent Cedar schemas. They differ
# only in where `type Ctx` is declared (top level vs. inside namespace App).
# Exit 1 if the bug shows (ok passes, bug fails), 0 if both pass, 2 otherwise.
cd "$(dirname "$0")" || exit 2
D="${1:?usage: repro.sh <dogwood-binary>}"
echo "== Ctx declared inside namespace App"
"$D" validate policy.dw --policy-schema ok.cedarschema; ok=$?
echo "== Ctx declared at top level"
"$D" validate policy.dw --policy-schema bug.cedarschema; bug=$?
if [ $ok -eq 0 ] && [ $bug -ne 0 ]; then exit 1; fi
if [ $ok -eq 0 ] && [ $bug -eq 0 ]; then exit 0; fi
exit 2
