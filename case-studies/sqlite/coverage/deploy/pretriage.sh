#!/usr/bin/env bash
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

# From cron every 30 minutes: pre-triage the current run's findings that
# have no verdict yet (pretriage.py, arguments passed on: --all for every
# run, --check-bugs for the known-bug check), against the CLIs build-clis.sh
# built. One pass at a time (a pass already running makes this one exit,
# unless PRETRIAGE_WAIT gives seconds to wait for it), niced, each statement
# under a timeout, a pass at most PRETRIAGE_BUDGET seconds (1500) per run;
# output in RUN_ROOT/pretriage.log.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

[ -e "$RUN_ROOT/clis/current/trunk/sqlite3" ] || { log "no CLIs in $RUN_ROOT/clis/current: run build-clis.sh"; exit 0; }
LOCK="$RUN_ROOT/.pretriage.lock"
if command -v flock > /dev/null; then
    exec 9> "$LOCK"
    if [ "${PRETRIAGE_WAIT:-0}" = 0 ]; then flock -n 9 || exit 0; else flock -w "$PRETRIAGE_WAIT" 9 || exit 0; fi
else
    # No flock (macOS): a lock directory, a dead pass's after two hours.
    WAITED=0
    until mkdir "$LOCK.d" 2> /dev/null; do
        [ -n "$(find "$LOCK.d" -maxdepth 0 -mmin +120 2> /dev/null)" ] && rmdir "$LOCK.d" && continue
        [ "$WAITED" -ge "${PRETRIAGE_WAIT:-0}" ] && exit 0
        sleep 5
        WAITED=$((WAITED + 5))
    done
    trap 'rmdir "$LOCK.d"' EXIT
fi
export RUN_ROOT STATSD TAGS CAMPAIGN TAG
{
    echo "== $(date -u +%Y-%m-%dT%H:%M:%SZ) pretriage $*"
    nice -n 10 python3 "$HERE/pretriage.py" --root "$RUN_ROOT" --bugs-root "$ROOT" \
        --timeout "${PRETRIAGE_TIMEOUT:-20}" --budget "${PRETRIAGE_BUDGET:-1500}" "$@"
} >> "$RUN_ROOT/pretriage.log" 2>&1 || log "pretriage $* failed (see $RUN_ROOT/pretriage.log)"
