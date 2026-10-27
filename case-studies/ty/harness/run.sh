#!/usr/bin/env bash
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

# run.sh <build-dir> <seed> <seconds> <out-dir>  (ephyra-agent harness contract)
# One tyz search: TYZ_WORKERS threads (default 6), each checking batches of TYZ_BATCH generated
# programs with CPython (bin/py) and then ty (bin/ty). Writes <out-dir>/stats.json after every batch
# and <out-dir>/findings/<signature>/<n>/{finding.json, repro.sh, original.py, tree.json}.
set -euo pipefail
BUILD="$(cd "$1" && pwd)"
mkdir -p "$4"
export TYZ_TY="$BUILD/bin/ty" TYZ_PY="$BUILD/bin/py"
export TYZ_WORKERS="${TYZ_WORKERS:-6}" TYZ_BATCH="${TYZ_BATCH:-40}"
exec "$BUILD/bin/tyz" search "$2" "$3" "$(cd "$4" && pwd)"
