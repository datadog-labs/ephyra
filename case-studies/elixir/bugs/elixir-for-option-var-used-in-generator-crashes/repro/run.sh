#!/usr/bin/env bash
# Usage: run.sh [path-to-elixir-binary]
# Exit 1 if the type checker crashes compiling repro.exs, 0 if it compiles and runs.
E=$(realpath "$(command -v "${1:-elixir}")") || exit 2
cd "$(dirname "$0")" || exit 2
out=$("$E" repro.exs 2>&1); echo "$out"
grep -q "found error while checking types" <<<"$out" && exit 1
grep -q "A.f(\[1, 2\]): " <<<"$out" && exit 0 || exit 2
