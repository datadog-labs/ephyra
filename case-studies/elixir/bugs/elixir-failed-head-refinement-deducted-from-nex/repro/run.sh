#!/usr/bin/env bash
# Usage: run.sh [path-to-elixir-binary]
# Exit 1 if the false "incompatible types given to A.f/1" warning appears, 0 if not.
cd "$(dirname "$0")" || exit 2
E=${1:-elixir}
out=$("$E" repro.exs 2>&1); echo "$out"
grep -q "incompatible types given to A.f/1" <<<"$out" && exit 1 || exit 0
