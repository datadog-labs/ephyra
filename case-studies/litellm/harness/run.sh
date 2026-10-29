#!/usr/bin/env bash
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

# run.sh <build-dir> <seed> <seconds> <out-dir>: one search process (no network needed).
set -euo pipefail
B=$(cd "$1" && pwd)
mkdir -p "$4"
OUT=$(cd "$4" && pwd)
source "$B/env.sh"
export LITELLM_LOG=ERROR
exec "$B/search" "$B" "$2" "$3" "$OUT" 2>>"$OUT/search.log"
