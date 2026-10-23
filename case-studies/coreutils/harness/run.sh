#!/usr/bin/env bash
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

# run.sh <build-dir> <seed> <seconds> <out-dir>: one search process (uudiff), no network.
# Optional: UUDIFF_UTILS=sort,cut to restrict the utilities.
set -euo pipefail
B=$(realpath "$1"); SEED=$2; SECS=$3; OUT=$4
GNU=${UUDIFF_GNU:-/agent-run/work/ref/gnu-9.12/bin}
mkdir -p "$OUT"
exec "$B/uudiff" "$B/bin" "$GNU" "$SEED" "$SECS" "$OUT" ${UUDIFF_UTILS:+"$UUDIFF_UTILS"}
