#!/usr/bin/env bash
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

# Keep the current run's supervisor alive: from cron at boot and every 5
# minutes. Does nothing when there is no current run, when it was stopped
# (its stop file), when it finished (past its deadline, no supervisor), or
# when its supervisor is running; else starts the supervisor again with the
# run's arguments. The run resumes from its own state (seed, deadline,
# counters, server-log offset); the supervisor starts the run's cluster
# again when it is down and kills workers a dead supervisor left.
#
# The supervisor starts under MEM_LIMIT_MB of address space (`ulimit -v`,
# Linux only), inherited by its workers and by the cluster it starts
# (pg_ctl, the postmaster, every backend): a parse that allocates faster
# than the supervisor's 500 ms RSS poll fails with "out of memory" there
# instead of taking the host's memory. Core files are off (`ulimit -c 0`):
# an assertion failure aborts a backend, and the crash record has the log.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

[ -e "$RUN_ROOT/current" ] || exit 0
DIR="$(cd "$RUN_ROOT/current" && pwd -P)"

# One watchdog at a time; a lock older than 10 minutes is a dead one's.
LOCK="$DIR/.watchdog.lock"
if ! mkdir "$LOCK" 2>/dev/null; then
    if [ -n "$(find "$LOCK" -maxdepth 0 -mmin +10 2>/dev/null)" ]; then rmdir "$LOCK"; fi
    exit 0
fi
trap 'rmdir "$LOCK"' EXIT

# A started supervisor removes the stop file, so it is checked before
# anything is started.
[ -f "$DIR/stop" ] && exit 0
PID=$(cat "$DIR/supervisor.pid" 2>/dev/null || true)
if [ -n "$PID" ] && kill -0 "$PID" 2>/dev/null && ps -o command= -p "$PID" | grep -q "runner run"; then
    exit 0
fi
if [ -f "$DIR/until" ] && [ "$(date +%s)" -ge "$(cat "$DIR/until")" ]; then
    exit 0
fi

ARGS=()
while IFS= read -r a; do ARGS+=("$a"); done < "$DIR/launch.args"
if [ -n "$PID" ]; then
    log "supervisor $PID of $DIR is gone: starting it again"
else
    log "starting the supervisor of $DIR"
fi
(
    ulimit -c 0 2>/dev/null || true
    if [ -n "${MEM_LIMIT_MB:-}" ] && [ "$(uname -s)" = Linux ]; then
        ulimit -v $((MEM_LIMIT_MB * 1024))
    fi
    exec nohup "$DIR/runner" run --dir "$DIR" "${ARGS[@]}" >> "$DIR/supervisor.log" 2>&1 < /dev/null
) &
