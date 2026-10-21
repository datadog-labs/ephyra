#!/usr/bin/env bash
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

# Build PostgreSQL master for the runner.
#
#     build-pg.sh [--print] [--no-fetch] [--coverage] [--ref REF]
#
# Clones PG_REPO into PG_SRC when it is missing (blobless), fetches master
# (unless --no-fetch), and builds REF (default: the fetched master) in a git
# worktree PG_BUILDS/<commit12>/ of PG_SRC, installed with contrib (amcheck,
# btree_gin, btree_gist; `make world-bin install-world-bin`) in its inst/.
# The worktree is a checkout, so the runner's target tag names its commit
# (`<checkout>/inst/bin`). Configured as ephyra-postgresql's development build,
# without ICU and readline: assertions on, so a crash is a finding.
#
# --coverage builds PG_BUILDS/<commit12>-cov/ instead, with clang's
# source-based coverage (postgresql-coverage's qscore build; needs clang):
# -O1, -fprofile-instr-generate -fcoverage-mapping, and libpq's "no exit()"
# check skipped (the profile runtime's atexit fails it; PostgreSQL's own
# --enable-coverage skips it too) by touching libpq-refs-stamp after
# building libpq.
#
# The first (non-coverage) build becomes PG_BUILDS/current when there is
# none. A build whose inst/.built exists is kept (FORCE=1 builds it again). Logs:
# build-configure.log, build-make.log, build-install.log in the build. The
# last line printed is the build's inst/bin. --print prints what would be
# done (the commit, the directories, the configure line) and touches
# nothing: no fetch, no worktree, no build.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

PRINT=0 FETCH=1 COVERAGE=0 REF=""
while [ $# -gt 0 ]; do
    case "$1" in
        --print) PRINT=1 ;;
        --no-fetch) FETCH=0 ;;
        --coverage) COVERAGE=1 ;;
        --ref) REF="$2"; shift ;;
        *) echo "usage: build-pg.sh [--print] [--no-fetch] [--coverage] [--ref REF]" >&2; exit 2 ;;
    esac
    shift
done

say() { echo "$*" >&2; }

if [ ! -d "$PG_SRC/.git" ] && [ ! -f "$PG_SRC/.git" ]; then
    if [ "$PRINT" = 1 ]; then
        say "would clone $PG_REPO into $PG_SRC"
        exit 0
    fi
    git clone --quiet --filter=blob:none "$PG_REPO" "$PG_SRC"
fi
if [ "$FETCH" = 1 ]; then
    if [ "$PRINT" = 1 ]; then
        say "would fetch master from $(git -C "$PG_SRC" remote get-url origin) (--print: using the local origin/master)"
    else
        git -C "$PG_SRC" fetch --quiet origin +master:refs/remotes/origin/master
    fi
fi
COMMIT=$(git -C "$PG_SRC" rev-parse --verify "${REF:-origin/master}^{commit}")
C12=$(echo "$COMMIT" | cut -c1-12)
NAME="$C12"
[ "$COVERAGE" = 1 ] && NAME="$C12-cov"
B="$PG_BUILDS/$NAME"
JOBS="${MAKE_JOBS:-$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)}"

CONFIGURE=(--prefix="$B/inst" --enable-cassert --enable-debug --without-icu --without-readline --with-lz4 --with-zstd)
if [ "$COVERAGE" = 1 ]; then
    ENVS=(CC=clang "CFLAGS=-O1 -g -fprofile-instr-generate -fcoverage-mapping" LDFLAGS=-fprofile-instr-generate)
else
    ENVS=("CFLAGS=-O2 -g")
fi

if [ "$PRINT" = 1 ]; then
    say "source:    $PG_SRC ($(git -C "$PG_SRC" log -1 --format='%h %cs %s' "$COMMIT"))"
    say "commit:    $COMMIT"
    say "build:     $B (git worktree of $PG_SRC, detached at $C12)"
    if [ -f "$B/inst/.built" ] && [ -z "${FORCE:-}" ]; then
        say "status:    already built; nothing to do"
    else
        say "configure: ./configure ${CONFIGURE[*]} ${ENVS[*]}"
        [ "$COVERAGE" = 1 ] && say "libpq:     make -C src/interfaces/libpq all-lib && touch src/interfaces/libpq/libpq-refs-stamp"
        say "make:      make -j$JOBS world-bin && make install-world-bin"
    fi
    echo "$B/inst/bin"
    exit 0
fi

if [ -f "$B/inst/.built" ] && [ -z "${FORCE:-}" ]; then
    echo "$B/inst/bin"
    exit 0
fi
mkdir -p "$PG_BUILDS"
if [ -e "$B" ]; then
    git -C "$PG_SRC" worktree remove --force "$B" 2>/dev/null || rm -rf "$B"
    git -C "$PG_SRC" worktree prune
fi
git -C "$PG_SRC" worktree add --quiet --detach "$B" "$COMMIT"
log "building PostgreSQL $C12 in $B"
cd "$B"
if ! env "${ENVS[@]}" ./configure "${CONFIGURE[@]}" > build-configure.log 2>&1; then
    log "configure failed for $C12: $B/build-configure.log"
    tail -20 build-configure.log >&2
    exit 1
fi
if [ "$COVERAGE" = 1 ]; then
    make -C src/interfaces/libpq -j"$JOBS" all-lib > build-make.log 2>&1
    touch src/interfaces/libpq/libpq-refs-stamp
fi
if ! make -j"$JOBS" world-bin >> build-make.log 2>&1; then
    log "make failed for $C12: $B/build-make.log"
    tail -20 build-make.log >&2
    exit 1
fi
if ! make install-world-bin > build-install.log 2>&1; then
    log "install failed for $C12: $B/build-install.log"
    tail -20 build-install.log >&2
    exit 1
fi
# pg_config's sharedir: PostgreSQL appends /postgresql to share/ and lib/
# unless the prefix contains "postgres" or "pgsql" (a prefix inside a
# checkout named postgres does, the default PG_BUILDS does not).
SHAREDIR=$("$B/inst/bin/pg_config" --sharedir)
for ext in amcheck btree_gin btree_gist; do
    if [ ! -f "$SHAREDIR/extension/$ext.control" ]; then
        log "$ext is not installed in $B/inst"
        exit 1
    fi
done
date -u +%Y-%m-%dT%H:%M:%SZ > "$B/inst/.built"
log "built PostgreSQL $C12: $("$B/inst/bin/postgres" -V)"
# The first build becomes the one runs use; refresh-master.sh moves it after.
if [ "$COVERAGE" = 0 ] && [ ! -e "$PG_BUILDS/current" ]; then
    ln -sfn "$B" "$PG_BUILDS/current"
    log "PG_BUILDS/current -> $B"
fi
echo "$B/inst/bin"
