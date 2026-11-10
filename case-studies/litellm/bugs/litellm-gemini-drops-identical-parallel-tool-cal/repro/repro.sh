#!/usr/bin/env bash
# repro.sh <build-dir>: exit 1 if litellm drops one of two identical parallel tool calls, 0 if not.
B=$(cd "$1" && pwd) || exit 2
cd "$(dirname "$0")" || exit 2
source "$B/env.sh" || exit 2
"$LITELLM_PY" poc.py
