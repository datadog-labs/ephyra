#!/usr/bin/env bash
# Usage: run.sh <path-to-elixir-binary>
# Exit 1 if the false "will never match" warning appears, 0 if not.
cd "$(dirname "$0")" || exit 2
E=${1:-elixir}
out=$("$E" repro.exs 2>&1); echo "$out"
grep -q "the following pattern will never match" <<<"$out" && exit 1 || exit 0
