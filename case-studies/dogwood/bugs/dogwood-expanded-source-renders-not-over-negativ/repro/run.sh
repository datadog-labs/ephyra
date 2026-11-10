#!/usr/bin/env bash
# Usage: run.sh <dogwood-checkout> [policy.dw]
# Builds a tiny program against <dogwood-checkout>/dogwood-language and checks
# that ParsedPolicy::expanded_source() still parses and re-lowers to the same Cedar policy.
# Exit 1: bug reproduces (rendered text fails to lower, or lowers differently). 0: lowerings match. Other: error.
set -u
here="$(cd "$(dirname "$0")" && pwd)"
checkout="$(cd "${1:?usage: run.sh <dogwood-checkout> [policy.dw]}" && pwd)" || exit 2
policy="${2:-$here/policy.dw}"
work="${TMPDIR:-/tmp}/not-neg-literal-poc.$$"
mkdir -p "$work/src" || exit 2
cp "$here/poc/src/main.rs" "$work/src/" || exit 2
sed "s#DOGWOOD_CHECKOUT#$checkout#" "$here/poc/Cargo.toml" > "$work/Cargo.toml" || exit 2
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$work/target}"
cargo build -q --manifest-path "$work/Cargo.toml" || exit 2
"$CARGO_TARGET_DIR/debug/not-neg-literal-poc" "$policy" "$here/schema.cedarschema"
