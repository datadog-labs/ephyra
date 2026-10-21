#!/usr/bin/env bash
# The gap list of a run (default: the current one): merge its workers'
# coverage profiles, export them with llvm-cov against the run's own binary,
# and summarize what SQLite code no input reached (gaps.py) into
# <run>/gaps/gaps.md and gaps.json.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
DIR="$(cd "${1:-$RUN_ROOT/current}" && pwd -P)"
tool() { if command -v "$1" > /dev/null; then echo "$1"; else echo "xcrun $1"; fi; }
PROFDATA=$(tool llvm-profdata)
COV_TOOL=$(tool llvm-cov)
$PROFDATA merge -sparse "$DIR"/workers/*/coverage-*.profraw -o "$DIR/coverage.profdata"
$COV_TOOL export -format=text -skip-expansions -instr-profile "$DIR/coverage.profdata" "$DIR/runner" \
    -sources "$VENDOR/sqlite3/sqlite3.c" > "$DIR/coverage.json"
python3 "$HERE/gaps.py" "$DIR/coverage.json" --sections "$COV/sections.tsv" --source "$VENDOR/sqlite3/sqlite3.c" --out "$DIR/gaps"
rm -f "$DIR/coverage.json"
log "gap list of $DIR: $DIR/gaps/gaps.md"
