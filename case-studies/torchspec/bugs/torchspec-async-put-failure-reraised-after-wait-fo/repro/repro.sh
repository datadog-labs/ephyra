#!/usr/bin/env bash
# Usage: repro.sh <build-dir>; exit 1 = reproduces, 0 = does not, other = undecided.
B=$(realpath "$1"); cd "$(dirname "$(realpath "$0")")" || exit 3
timeout -k 5 60 env HF_HUB_OFFLINE=1 "$B/venv/bin/python" -I check_repro.py
rc=$?
case $rc in 0|1) exit $rc ;; *) exit 3 ;; esac
