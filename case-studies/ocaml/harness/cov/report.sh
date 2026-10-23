#!/bin/bash
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

# Wrapper around bisect-ppx-report: `cobertura` prints source paths relative to
# the current directory when they lie under it (e.g. cwd=work gives
# "trees/cov/asmcomp/x.ml"), which hides the absolute tree paths the controller
# matches on. Run the real tool from / with every relative path argument made
# absolute.
REAL="$(cd "$(dirname "$0")" && pwd)/bisect-ppx-report"
args=("$1"); shift   # subcommand
for a in "$@"; do
  case "$a" in
    -*|/*) args+=("$a") ;;
    *) args+=("$PWD/$a") ;;
  esac
done
cd / && exec "$REAL" "${args[@]}"
