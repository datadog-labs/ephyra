#!/usr/bin/env bash
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

# run.sh <build-dir> <seed> <seconds> <out-dir>: one search process (BRUSHFUZZ_THREADS workers, default 3).
set -euo pipefail
B=$(realpath "$1")
mkdir -p "$4"
exec "$B/brushfuzz" run "$B" "$2" "$3" "$(realpath "$4")"
