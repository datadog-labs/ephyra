#!/usr/bin/env bash
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

# run.sh <build-dir> <seed> <seconds> <out-dir>
set -euo pipefail
BUILD="$(cd "$1" && pwd)"
mkdir -p "$4"
exec "$BUILD/bin/dfz" search "$2" "$3" "$4"
