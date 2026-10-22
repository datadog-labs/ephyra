#!/usr/bin/env bash
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

# Nightly: move the run to PostgreSQL master's newest commit.
#
# 1. build-pg.sh fetches master and builds it (PG_BUILDS/<commit12>/); when
#    its commit is the current build's, stops (FORCE=1 goes on anyway).
# 2. Runs ephyra-postgresql's `cargo test --lib` and `cargo test --test
#    cases` against the new build (EPHYRA_PG_BIN; a cluster of its own,
#    RUN_ROOT/t-<commit12>, stopped and removed afterwards). When they fail,
#    the run stays on the old build (the log names the failing tests) unless
#    SWITCH_ON_TEST_FAILURE=1: a test failing on a new master is either a
#    PostgreSQL change the generator or model must follow, or a bug, and
#    either would flood the run with findings.
# 3. Stops the current run (its stop file; the supervisor gives workers 90 s
#    and stops the cluster), points PG_BUILDS/current at the new build and
#    starts a new run (start-run.sh). When the new run is not up within 5
#    minutes (supervisor alive, cluster accepting connections, a worker
#    started), points PG_BUILDS/current back at the old build and starts a
#    run on it.
# 4. Once the new run is up, removes builds beyond the newest KEEP_BUILDS
#    (never the current one): the old build stays for replays of the old
#    run's findings.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

# One refresh at a time.
# The lock is fd 9: the runs started below get it closed (9>&-), or the
# supervisor and its workers would inherit it and hold it for the run's life,
# and every later refresh would find "another refresh is running" (as on
# 2026-10-18).
exec 9> "$RUN_ROOT/.refresh.lock"
if command -v flock > /dev/null && ! flock -n 9; then
    log "another refresh is running"
    exit 0
fi

OUT="$RUN_ROOT/refresh-build.log"
if ! NEW_BIN=$("$HERE/build-pg.sh" 2> "$OUT" | tail -1); then
    log "build-pg.sh failed: $OUT; the run stays on its build"
    exit 1
fi
NEW=$(pg_commit "$NEW_BIN")
OLD_BUILD=""
OLD=""
if [ -e "$PG_BUILDS/current" ]; then
    OLD_BUILD="$(cd "$PG_BUILDS/current" && pwd -P)"
    OLD=$(pg_commit "$OLD_BUILD/inst/bin")
fi
if [ "$NEW" = "$OLD" ] && [ -z "${FORCE:-}" ]; then
    log "master unchanged ($OLD)"
    exit 0
fi
log "master moved: ${OLD:-none} -> $NEW"

TESTS="$RUN_ROOT/refresh-$NEW-tests.log"
TDIR="$RUN_ROOT/t-$NEW"
rm -rf "$TDIR"
PASS=1
(
    cd "$EPG"
    export EPHYRA_PG_BIN="$NEW_BIN" EPHYRA_PG_DIR="$TDIR"
    cargo test --lib && cargo test --test cases
) > "$TESTS" 2>&1 || PASS=0
"$NEW_BIN/pg_ctl" -D "$TDIR/data" stop -m fast > /dev/null 2>&1 || true
rm -rf "$TDIR"
if [ "$PASS" = 1 ]; then
    log "ephyra-postgresql tests pass on $NEW"
else
    FAILED=$(grep -E '^test .* FAILED|^error' "$TESTS" | tr '\n' ' ' || true)
    if [ "${SWITCH_ON_TEST_FAILURE:-0}" != 1 ]; then
        log "ephyra-postgresql tests fail on $NEW ($TESTS): ${FAILED:-see the log}; the run stays on ${OLD:-its build}"
        exit 1
    fi
    log "ephyra-postgresql tests fail on $NEW ($TESTS): ${FAILED:-see the log}; moving anyway (SWITCH_ON_TEST_FAILURE=1)"
fi

stop_current() {
    [ -e "$RUN_ROOT/current" ] || return 0
    local dir
    dir="$(cd "$RUN_ROOT/current" && pwd -P)"
    touch "$dir/stop"
    for _ in $(seq 120); do
        [ -f "$dir/supervisor.pid" ] || break
        sleep 5
    done
    if [ -f "$dir/supervisor.pid" ]; then
        log "the supervisor of $dir did not stop in 10 minutes; killing it"
        kill -9 "$(cat "$dir/supervisor.pid")" 2>/dev/null || true
        rm -f "$dir/supervisor.pid"
    fi
    # The cluster, if the supervisor could not stop it.
    local pgd
    pgd="$(cat "$dir/pg_dir" 2>/dev/null || echo "$dir/pg")"
    if [ -f "$pgd/data/postmaster.pid" ]; then
        "$(awk 'p { print; exit } $0 == "--pg-bin" { p = 1 }' "$dir/launch.args")/pg_ctl" -D "$pgd/data" stop -m immediate > /dev/null 2>&1 || true
    fi
    log "stopped $dir"
}

# Up: supervisor alive, its cluster accepting connections, a worker started.
run_up() {
    local dir pid
    dir="$(cd "$RUN_ROOT/current" && pwd -P)"
    for _ in $(seq 60); do
        sleep 5
        pid=$(cat "$dir/supervisor.pid" 2>/dev/null || true)
        if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null \
            && "$1/pg_isready" -q -h "$dir/pg" > /dev/null 2>&1 \
            && grep -q '"kind":"worker_start"' "$dir/events.jsonl" 2>/dev/null; then
            return 0
        fi
    done
    return 1
}

stop_current
ln -sfn "$(cd "$NEW_BIN/../.." && pwd -P)" "$PG_BUILDS/current"
# PG_TARGET=fixed: the new master with the known bugs' fixes. When that
# build fails, PG_BUILDS/fixed keeps pointing at the old one.
if [ "${PG_TARGET:-master}" = fixed ]; then
    "$HERE/build-fixed.sh" > /dev/null 2>> "$OUT" || log "build-fixed.sh failed on $NEW ($OUT); the run starts on the previous fixed build"
fi
# Pre-triage replays on the new master and its single-fix builds: without
# them it has no master at all once the old build is removed below (as on
# 2026-10-16, when every pass failed with "no master build").
"$HERE/build-patched.sh" > /dev/null 2>> "$OUT" || log "build-patched.sh failed on $NEW ($OUT); pre-triage has no builds for it"
if "$HERE/start-run.sh" 9>&- && run_up "$NEW_BIN"; then
    log "new run on $NEW is up"
else
    log "the new run on $NEW did not come up: back to ${OLD:-nothing}"
    stop_current
    if [ -n "$OLD_BUILD" ]; then
        ln -sfn "$OLD_BUILD" "$PG_BUILDS/current"
        sleep 60   # start-run.sh names runs by the minute
        "$HERE/start-run.sh" 9>&- || log "the run on $OLD did not start either"
    fi
    exit 1
fi

# Keep the current build and the newest KEEP_BUILDS (coverage builds are
# build-pg.sh --coverage's own and are left alone).
CUR="$(cd "$PG_BUILDS/current" && pwd -P)"
FIX="$(cd "$PG_BUILDS/fixed" 2>/dev/null && pwd -P || true)"
# shellcheck disable=SC2012
ls -1dt "$PG_BUILDS"/[0-9a-f]*/ 2>/dev/null | sed 's:/$::' | grep -v -- '-cov$' | tail -n +"$((KEEP_BUILDS + 1))" | while read -r b; do
    [ "$(cd "$b" && pwd -P)" = "$CUR" ] && continue
    [ "$(cd "$b" && pwd -P)" = "$FIX" ] && continue
    git -C "$PG_SRC" worktree remove --force "$b" 2>/dev/null || rm -rf "$b"
    log "removed old build $b"
done
git -C "$PG_SRC" worktree prune
