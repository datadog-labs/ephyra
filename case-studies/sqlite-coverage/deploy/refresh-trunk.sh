#!/usr/bin/env bash
# Nightly: move to SQLite trunk's newest check-in. Builds trunk's
# amalgamation and stops when its check-in is the vendored one (FORCE=1
# goes on anyway). Else re-vendors it (the old files kept in
# vendor/backup-<check-in>/) and regenerates sections.tsv from it, runs
# ephyra-sqlite's tests against it (a
# fail-when-fixed test failing means a SQLite bug was fixed and its
# exclusion can go; the log says which), builds the pre-triage CLIs from the
# same tree (build-clis.sh: trunk and every open known bug's fix), checks the
# known bugs on the new trunk (pretriage.py --check-bugs: a bug's check
# printing its correct output is fixed upstream, event known_bug_fixed; a fix
# that no longer applies, patch_stale; RUN_ROOT/known-bugs-status.json),
# pre-triages the current run's findings with the new CLIs, stops the
# current run, starts a new one on the new trunk, and pre-triages that.
#
# SQLITE_TARGET=fixed (runner.env): what is vendored, and so what the new run
# tests, is build-fixed.sh's amalgamation of the same tree with every open
# known bug's fix applied (its fixes.json beside the vendored files), not
# plain trunk; sections.tsv and ephyra-sqlite's tests follow it. The
# pre-triage CLIs and the known-bug check stay plain trunk's and each
# bug's, and build-fixed.sh adds RUN_ROOT/clis/<check-in>/fixed/. When the
# fixed build fails, plain trunk is vendored (logged). Switching the target
# without a trunk move: FORCE=1 refresh-trunk.sh.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
curl -sfL https://sqlite.org/src/tarball/trunk/sqlite.tar.gz -o "$TMP/sqlite.tar.gz"
tar xzf "$TMP/sqlite.tar.gz" -C "$TMP"
(cd "$TMP/sqlite" && ./configure > "$TMP/configure.log" 2>&1 && make sqlite3.c > "$TMP/make.log" 2>&1)
NEW=$(checkin "$TMP/sqlite/sqlite3.h")
OLD=$(checkin "$VENDOR/sqlite3/sqlite3.h")
if [ "$NEW" = "$OLD" ] && [ -z "${FORCE:-}" ]; then
    log "trunk unchanged ($OLD)"
    exit 0
fi
log "trunk moved: $OLD -> $NEW"

# What to vendor: plain trunk, or with SQLITE_TARGET=fixed trunk with the
# known bugs' fixes (falling back to plain trunk when that build fails).
FIXED=""
if [ "${SQLITE_TARGET:-master}" = fixed ]; then
    if FIXED=$("$HERE/build-fixed.sh" "$TMP/sqlite" 2>> "$RUN_ROOT/deploy.log" | tail -1) && [ -f "$FIXED/fixes.json" ]; then
        log "vendoring $NEW with the known bugs' fixes ($FIXED/fixes.json)"
    else
        log "build-fixed.sh failed on $NEW: vendoring plain trunk"
        FIXED=""
    fi
fi

mkdir -p "$VENDOR/backup-$OLD"
cp "$VENDOR/sqlite3/sqlite3.c" "$VENDOR/sqlite3/sqlite3.h" "$VENDOR/sqlite3/sqlite3ext.h" "$VENDOR/backup-$OLD/"
if [ -f "$VENDOR/sqlite3/fixes.json" ]; then cp "$VENDOR/sqlite3/fixes.json" "$VENDOR/backup-$OLD/"; fi
if [ -n "$FIXED" ]; then
    cp "$FIXED/sqlite3.c" "$FIXED/sqlite3.h" "$FIXED/sqlite3ext.h" "$FIXED/fixes.json" "$VENDOR/sqlite3/"
else
    cp "$TMP/sqlite/sqlite3.c" "$TMP/sqlite/sqlite3.h" "$VENDOR/sqlite3/"
    cp "$TMP/sqlite/src/sqlite3ext.h" "$VENDOR/sqlite3/"
    rm -f "$VENDOR/sqlite3/fixes.json"
fi
ID=$(grep -m1 'define SQLITE_SOURCE_ID' "$VENDOR/sqlite3/sqlite3.h" | awk '{print $5}' | cut -c1-12)
WHEN=$(grep -m1 'define SQLITE_SOURCE_ID' "$VENDOR/sqlite3/sqlite3.h" | awk '{print $3" "$4}' | tr -d '"')
# vendor/ is case-studies/vendor/fetch.sh's, not tracked; when it has a
# README.md naming the vendored check-in, keep that current (without one
# this sed fails, which does not stop the script).
sed -i.bak -E "s/check-in [0-9a-f]{12} \([0-9-]+ [0-9:]+\)/check-in $ID ($WHEN)/" "$SQL/vendor/README.md" && rm -f "$SQL/vendor/README.md.bak"
python3 "$HERE/sections.py" >> "$RUN_ROOT/deploy.log" 2>&1

TESTS="$RUN_ROOT/refresh-$NEW-tests.log"
(cd "$SQL/ephyra-sqlite" && cargo clean -p libsqlite3-sys && cargo test) > "$TESTS" 2>&1 \
    && log "ephyra-sqlite tests pass on $NEW${FIXED:+ with the known bugs' fixes}" \
    || log "ephyra-sqlite tests fail on $NEW${FIXED:+ with the known bugs' fixes} (a fixed SQLite bug's exclusion${FIXED:+, one of our fixes among them}, or a change): $(grep -E '^test .* FAILED' "$TESTS" | tr '\n' ' ')"
(cd "$COV" && cargo clean --release -p libsqlite3-sys) >> "$RUN_ROOT/deploy.log" 2>&1

"$HERE/build-clis.sh" "$TMP/sqlite" >> "$RUN_ROOT/deploy.log" 2>&1 || log "build-clis.sh failed on $NEW"
PRETRIAGE_WAIT=1800 "$HERE/pretriage.sh" --check-bugs
PRETRIAGE_WAIT=1800 "$HERE/pretriage.sh"

if [ -e "$RUN_ROOT/current" ]; then
    DIR="$(cd "$RUN_ROOT/current" && pwd -P)"
    touch "$DIR/stop"
    for _ in $(seq 120); do
        [ -f "$DIR/supervisor.pid" ] || break
        sleep 5
    done
    log "stopped $DIR for the new trunk"
fi
"$HERE/start-run.sh"
PRETRIAGE_WAIT=1800 "$HERE/pretriage.sh"
