#!/bin/sh
# usage: repro.sh <python-with-fastmcp>   exit 1 = legacy client served a stale cached tools/list
cd "$(dirname "$0")" || exit 2
out=$(FASTMCP_SHOW_SERVER_BANNER=false "${1:-python}" -I repro.py 2>&1); echo "$out"
case "$out" in *"after:  ['first', 'second']"*) exit 0 ;; *"after:  ['first']"*) exit 1 ;; *) exit 2 ;; esac
