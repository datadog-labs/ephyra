#!/usr/bin/env bash
# Usage: run.sh [path-to-elixir-binary]
# Exit 1 if a "will never match" warning appears (A.f() returns 1, so the first clause matches), 0 if not.
E=$(realpath "$(command -v "${1:-elixir}")") || exit 2
cd "$(dirname "$0")" || exit 2
out=$("$E" repro.exs 2>&1); echo "$out"
grep -q "will never match" <<<"$out" && exit 1 || exit 0
