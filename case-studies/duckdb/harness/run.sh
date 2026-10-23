#!/bin/bash
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

# run.sh <build-dir> <seed> <seconds> <out-dir>: one search process (no network).
set -euo pipefail
B=$(realpath "$1"); SEED=$2; SECS=$3; mkdir -p "$4"; OUT=$(realpath "$4")
export LD_LIBRARY_PATH="$B/cmake/src"
exec "$B/duckharness" search "$SEED" "$SECS" "$OUT"
