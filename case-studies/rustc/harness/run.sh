#!/bin/sh
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

# run.sh <build-dir> <seed> <seconds> <out-dir>   (see run.py / README.md)
HERE=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$4"
PYTHONDONTWRITEBYTECODE=1 exec python3 "$HERE/run.py" "$@"
