#!/usr/bin/env bash
# Start a new run: build the runner (and check that the binary is built
# from the sources on disk), make a run directory named after the date and
# the PostgreSQL commit (RUN_ROOT/<date>-postgresql-<commit12>/), copy the
# binary into it (so that rebuilds never touch a running binary), write its
# arguments (binaries pinned with --pg-bin to the resolved build, so that
# refresh-master.sh moving PG_BUILDS/current never changes a running run),
# point RUN_ROOT/current at it, and start it through the watchdog (which
# sets the memory limit, MEM_LIMIT_MB).
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

BIN=$(pg_bin) || { log "no PostgreSQL build (PG_BIN or PG_BUILDS/current): run build-pg.sh"; exit 1; }
for b in postgres initdb pg_ctl pg_isready; do
    [ -x "$BIN/$b" ] || { log "$BIN/$b is missing; not starting"; exit 1; }
done
COMMIT=$(pg_commit "$BIN")
if [ -z "$COMMIT" ]; then
    log "$BIN is not <checkout>/inst/bin of a git checkout (the target would be unknown); not starting"
    exit 1
fi

(cd "$COV" && cargo build --release --bin runner) >> "$RUN_ROOT/deploy.log" 2>&1
# Cargo rebuilds by modification times, and copied sources can keep older
# ones: when the binary was built from other sources than those on disk,
# touch them and build again, and never start a stale binary.
if ! "$COV/target/release/runner" version >> "$RUN_ROOT/deploy.log" 2>&1; then
    log "stale runner binary: touching the sources and rebuilding"
    find "$SQL/../crates/ephyra" "$EPG" "$COV" -path '*/target' -prune -o \
        \( -name '*.rs' -o -name Cargo.toml -o -name Cargo.lock \) -type f -exec touch {} +
    (cd "$COV" && cargo build --release --bin runner) >> "$RUN_ROOT/deploy.log" 2>&1
    if ! "$COV/target/release/runner" version >> "$RUN_ROOT/deploy.log" 2>&1; then
        log "the runner binary is still stale; not starting"
        exit 1
    fi
fi

DIR="$RUN_ROOT/$(date -u +%Y%m%d-%H%M)-postgresql-$COMMIT"
SOCKET="$DIR/pg/.s.PGSQL.5432"
if [ "${#SOCKET}" -ge 100 ]; then
    log "the cluster's socket $SOCKET is ${#SOCKET} bytes, a Unix socket path takes under 100: set a shorter RUN_ROOT"
    exit 1
fi
if [ -e "$DIR" ]; then
    log "$DIR exists (a run started this minute); not starting another"
    exit 1
fi
mkdir -p "$DIR"
cp "$COV/target/release/runner" "$DIR/runner"
{
    echo --workers; echo "$WORKERS"
    echo --hours; echo "$HOURS"
    echo --campaign; echo "$CAMPAIGN"
    echo --tag; echo "$TAG"
    echo --max-rss-mb; echo "$MAX_RSS_MB"
    echo --stale-secs; echo "$STALE_SECS"
    echo --pg-bin; echo "$BIN"
    # build-fixed.sh's build: the bugs it has fixed.
    if [ -f "$BIN/../../fixes.json" ]; then
        echo --fixed-bugs; python3 -c 'import json, sys; print(",".join(json.load(open(sys.argv[1]))["applied"]))' "$BIN/../../fixes.json"
    fi
    if [ -n "$STATSD" ]; then echo --statsd; echo "$STATSD"; fi
    if [ -n "$TAGS" ]; then echo --tags; echo "$TAGS"; fi
} > "$DIR/launch.args"
ln -sfn "$DIR" "$RUN_ROOT/current"
log "new run $DIR (PostgreSQL $COMMIT, $BIN)"
exec "$HERE/watchdog.sh"
