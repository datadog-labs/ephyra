#!/usr/bin/env bash
# Usage: run.sh [path-to-elixir-binary]
# Exit 1 if the false "comparison between distinct types" warning appears while A.f(<<1::1>>)
# returns :other, 0 if no such warning, 2 if it did not run.
E=$(realpath "$(command -v "${1:-elixir}")") || exit 2
cd "$(dirname "$0")" || exit 2
out=$("$E" repro.exs 2>&1); echo "$out"
grep -q '^A.f(<<1::1>>): :other' <<<"$out" || exit 2
grep -q "comparison between distinct types" <<<"$out" && exit 1 || exit 0
