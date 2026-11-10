#!/usr/bin/env bash
# repro.sh <build-dir>: exit 1 if the bug reproduces, 0 if not, 2 if undecided.
# <build-dir>/env.sh must set LITELLM_PY (a python with litellm importable).
# Runs offline: poc.py starts its own mock Gemini server on 127.0.0.1.
B=$(cd "$1" && pwd) || exit 2
cd "$(dirname "$0")" || exit 2
source "$B/env.sh" || exit 2
"$LITELLM_PY" poc.py
case $? in 10) exit 1 ;; 20) exit 0 ;; *) exit 2 ;; esac
