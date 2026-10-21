#!/usr/bin/env bash
# From cron every 30 minutes: pre-triage the current run's findings that
# have no verdict yet (pretriage.py; arguments passed on: run or findings
# directories, --force, --check-bugs for the known-bug check, --expected),
# against the builds build-patched.sh made (PG_PATCHED, default
# PG_BUILDS/patched, its current). One pass at a time (a pass already
# running makes this one exit, unless PRETRIAGE_WAIT gives seconds to wait
# for it), niced, a pass at most PRETRIAGE_BUDGET seconds (1500), each
# replay at most PRETRIAGE_REPLAY_SECS (180) and PRETRIAGE_MAX_RSS_MB (4096)
# per backend, throwaway clusters on ports PRETRIAGE_PORT_BASE (5641) and
# up; output in RUN_ROOT/pretriage.log, a summary line per finding in
# RUN_ROOT/deploy.log.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

PATCHED="${PG_PATCHED:-$PG_BUILDS/patched}"
[ -e "$PATCHED/current/status.json" ] || { log "no builds in $PATCHED/current: run build-patched.sh"; exit 0; }
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
export RUN_ROOT
{
    echo "== $(date -u +%Y-%m-%dT%H:%M:%SZ) pretriage $*"
    nice -n 10 python3 "$HERE/pretriage.py" --root "$RUN_ROOT" --builds "$PATCHED/current" \
        --pg-builds "$PG_BUILDS" --pg-src "$PG_SRC" --runner "$COV/target/release/runner" \
        --port-base "${PRETRIAGE_PORT_BASE:-5641}" --budget "${PRETRIAGE_BUDGET:-1500}" \
        --replay-secs "${PRETRIAGE_REPLAY_SECS:-180}" --max-rss-mb "${PRETRIAGE_MAX_RSS_MB:-4096}" "$@"
} >> "$RUN_ROOT/pretriage.log" 2>&1 || log "pretriage $* failed (see $RUN_ROOT/pretriage.log)"
