#!/usr/bin/env bash
# Usage: run.sh [path-to-elixir-binary]
# Exit 1 if any type warning appears (A.f(2) runs fine and returns 2), 0 if not.
E=$(realpath "$(command -v "${1:-elixir}")") || exit 2
cd "$(dirname "$0")" || exit 2
out=$("$E" repro.exs 2>&1); echo "$out"
grep -q "warning:" <<<"$out" && exit 1 || exit 0
