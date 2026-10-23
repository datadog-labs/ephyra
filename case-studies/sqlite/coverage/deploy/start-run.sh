#!/usr/bin/env bash
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

# Start a new run: build the runner (and check that the binary is built
# from the sources on disk), make a run directory named after the
# date and the vendored SQLite's check-in, copy the binary into it (so that
# rebuilds never touch a running binary), write its arguments, point
# RUN_ROOT/current at it, and start it through the watchdog. When the
# vendored amalgamation is build-fixed.sh's (its fixes.json beside it names
# sqlite3.c's sha256), the run gets `--fixed-bugs` with the fixes it has
# (in its run.json's config and run_start event).
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

(cd "$COV" && cargo build --release --bin runner) >> "$RUN_ROOT/deploy.log" 2>&1
# Cargo rebuilds by modification times, and copied sources can keep older
# ones: when the binary was built from other sources than those on disk,
# touch them and build again, and never start a stale binary.
if ! "$COV/target/release/runner" version >> "$RUN_ROOT/deploy.log" 2>&1; then
    log "stale runner binary: touching the sources and rebuilding"
    find "$ROOT/crates/ephyra" "$TARGET/generator" "$COV" -path '*/target' -prune -o \
        \( -name '*.rs' -o -name Cargo.toml -o -name Cargo.lock \) -type f -exec touch {} +
    (cd "$COV" && cargo build --release --bin runner) >> "$RUN_ROOT/deploy.log" 2>&1
    if ! "$COV/target/release/runner" version >> "$RUN_ROOT/deploy.log" 2>&1; then
        log "the runner binary is still stale; not starting"
        exit 1
    fi
fi
CHECKIN=$(checkin "$VENDOR/sqlite3/sqlite3.h")
# The known bugs whose fixes the vendored SQLite has (build-fixed.sh's
# fixes.json, when it is this sqlite3.c's).
FIXED_BUGS=""
if [ -f "$VENDOR/sqlite3/fixes.json" ]; then
    FIXED_BUGS=$(python3 -c '
import json, sys
j = json.load(open(sys.argv[1]))
if j.get("sqlite3_c_sha256") == sys.argv[2]:
    print(",".join(j["applied"]))
' "$VENDOR/sqlite3/fixes.json" "$(sha256 "$VENDOR/sqlite3/sqlite3.c")")
    [ -n "$FIXED_BUGS" ] || log "warning: $VENDOR/sqlite3/fixes.json is not the vendored sqlite3.c's; the run records no fixes"
fi
if [ "${SQLITE_TARGET:-master}" = fixed ] && [ -z "$FIXED_BUGS" ]; then
    log "warning: SQLITE_TARGET=fixed, but the vendored SQLite $CHECKIN is plain trunk (FORCE=1 refresh-trunk.sh builds the fixed one)"
elif [ "${SQLITE_TARGET:-master}" != fixed ] && [ -n "$FIXED_BUGS" ]; then
    log "warning: SQLITE_TARGET is not fixed, but the vendored SQLite $CHECKIN has the fixes of $FIXED_BUGS (FORCE=1 refresh-trunk.sh vendors plain trunk)"
fi
DIR="$RUN_ROOT/$(date -u +%Y%m%d-%H%M)-sqlite-$CHECKIN"
mkdir -p "$DIR"
cp "$COV/target/release/runner" "$DIR/runner"
{
    if [ -n "${PROFILES:-}" ]; then
        echo --profiles; echo "$PROFILES"
    else
        echo --fresh; echo "$FRESH"
        echo --targeted; echo "$TARGETED"
    fi
    echo --hours; echo "$HOURS"
    echo --reference; echo "$REFERENCE"
    echo --campaign; echo "$CAMPAIGN"
    echo --tag; echo "$TAG"
    if [ -n "${QUERIES:-}" ]; then echo --queries; echo "$QUERIES"; fi
    echo --max-rss-mb; echo "$MAX_RSS_MB"
    echo --stale-secs; echo "$STALE_SECS"
    if [ -n "$FIXED_BUGS" ]; then echo --fixed-bugs; echo "$FIXED_BUGS"; fi
    if [ -n "$STATSD" ]; then echo --statsd; echo "$STATSD"; fi
    if [ -n "$TAGS" ]; then echo --tags; echo "$TAGS"; fi
} > "$DIR/launch.args"
ln -sfn "$DIR" "$RUN_ROOT/current"
log "new run $DIR (SQLite $CHECKIN${FIXED_BUGS:+ with the fixes of $FIXED_BUGS})"
exec "$HERE/watchdog.sh"
