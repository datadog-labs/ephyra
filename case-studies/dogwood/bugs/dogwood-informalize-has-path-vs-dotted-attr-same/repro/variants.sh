#!/usr/bin/env bash
# Usage: variants.sh <path-to-dogwood-binary>
# Prints the `when:` line of `dogwood explain` for each condition,
# as `permit (principal, action, resource) when { <cond> };`.
set -u
here="$(cd "$(dirname "$0")" && pwd)"
D="${1:?usage: variants.sh <path-to-dogwood-binary>}"
t="$(mktemp -d)"; trap 'rm -rf "$t"' EXIT
for c in \
  'context has input.user' 'context has "input.user"' \
  'principal has a.b' 'principal has "a.b"' \
  'context.input has user' 'context.input has "user"' \
  'context.input.user == "x"' 'context["input.user"] == "x"'; do
  printf 'permit (principal, action, resource)\nwhen { %s };\n' "$c" > "$t/p.dw"
  printf '%-32s => %s\n' "$c" "$("$D" explain "$t/p.dw" --policy-schema "$here/schema.cedarschema" | tail -n 1 | sed 's/^ *//')"
done
