#!/bin/bash
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

# run.sh <build-dir> <seed> <seconds> <out-dir>: one search process.
# Writes <out-dir>/stats.json (periodically) and <out-dir>/findings/<sig>/<n>/.
# The environment (LLVM_PROFILE_FILE for coverage builds) is passed down to wasm-opt.
set -u
BUILD=$(realpath "$1")
mkdir -p "$4"
exec "$BUILD/bin/bynfuzz" "$BUILD" "$2" "$3" "$4"
