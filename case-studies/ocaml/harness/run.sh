#!/bin/bash
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

# run.sh <build-dir> <seed> <seconds> <out-dir>
set -euo pipefail
B=$(realpath "$1")
mkdir -p "$4"
# core dumps of crashing programs take seconds each (and turned a bytecode crash into a timeout)
ulimit -c 0
exec "$B/bin/ocfuzz" search --build "$B" --seed "$2" --seconds "$3" --out "$(realpath "$4")"
