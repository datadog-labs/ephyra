#!/usr/bin/env bash
# Usage: repro.sh <path-to-brush> [path-to-bash]
# Runs script.sh (next to this file) with brush and, optionally, bash, and shows stderr.
# Exit 1 = bug reproduces (brush writes to stderr even though the group's stderr is /dev/null),
# 0 = does not reproduce, 2 = usage error.
cd "$(dirname "$0")" || exit 2
BRUSH=${1:?usage: repro.sh <path-to-brush> [path-to-bash]}
BASH_REF=${2:-}

if [ -n "$BASH_REF" ]; then
  echo "=== bash ($("$BASH_REF" --version | head -1))"
  "$BASH_REF" --norc --noprofile script.sh; echo "exit: $?"
fi

echo "=== brush ($("$BRUSH" --version | head -1))"
err=$("$BRUSH" --norc --noprofile script.sh 2>&1 >/dev/null); code=$?
printf 'stderr: %s\nexit: %s\n' "${err:-<empty>}" "$code"

if [ -n "$err" ]; then
  echo "REPRODUCES: error escaped the group's 2>/dev/null"
  exit 1
fi
echo "does not reproduce"
exit 0
