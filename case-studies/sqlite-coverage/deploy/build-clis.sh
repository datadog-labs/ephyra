#!/usr/bin/env bash
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

# Build the sqlite3 CLIs pre-triage replays findings with: from a SQLite
# source tree (SRC_TREE, or else the tarball of the vendored check-in,
# downloaded), RUN_ROOT/clis/<check-in>/trunk/sqlite3 and, for every open
# bug of known-bugs.json with a fix, RUN_ROOT/clis/<check-in>/<bug>/sqlite3
# with that fix applied. Each is built in a fresh copy of the tree's files
# (those its manifest lists, so never a configured tree's build products)
# that is configured inside itself (a copied tree's Makefile names the
# original as TOP), as `make sqlite3.c shell.c` and then the CLI compiled
# with the runner's own SQLite options (vendor/libsqlite3-sys-trunk's
# build.rs: foreign keys on, STAT4, ...), all in parallel. A fix that no
# longer applies is logged and its status is patch_stale; the others go
# on. <bug>/status says ok, patch_stale or build_failed (build.log beside
# it), status.json all of them. A CLI already built from the same fix is
# kept (FORCE=1 rebuilds), so adding a bug builds only it. Points
# RUN_ROOT/clis/current at the newest check-in that has a trunk CLI and
# keeps the newest CLIS_KEEP (30) check-ins.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

REGISTRY="${KNOWN_BUGS:-$HERE/known-bugs.json}"
CLIS="$RUN_ROOT/clis"
mkdir -p "$CLIS"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

if [ $# -ge 1 ]; then
    SRC="$(cd "$1" && pwd -P)"
else
    WANT=trunk
    [ -f "$VENDOR/sqlite3/sqlite3.h" ] && WANT=$(checkin "$VENDOR/sqlite3/sqlite3.h")
    curl -sfL "https://sqlite.org/src/tarball/$WANT/sqlite.tar.gz" -o "$TMP/sqlite.tar.gz"
    mkdir "$TMP/download"
    tar xzf "$TMP/sqlite.tar.gz" -C "$TMP/download"
    SRC="$TMP/download/sqlite"
fi
[ -f "$SRC/manifest" ] && [ -f "$SRC/manifest.uuid" ] || { log "no manifest in $SRC: not a SQLite source tree"; echo "no manifest in $SRC" >&2; exit 1; }
CHECKIN=$(cut -c1-12 "$SRC/manifest.uuid")
OUT="$CLIS/$CHECKIN"
mkdir -p "$OUT"
if [ -f "$VENDOR/sqlite3/sqlite3.h" ] && [ "$(checkin "$VENDOR/sqlite3/sqlite3.h")" != "$CHECKIN" ]; then
    log "warning: CLIs for $CHECKIN, but the vendored amalgamation is $(checkin "$VENDOR/sqlite3/sqlite3.h")"
fi

# The tree's own files: the manifest's, and the manifest itself.
tree_files "$SRC" > "$TMP/files"

CC="${CC:-cc}"

# name, fix (absolute, or "-" for trunk), strip, dir: trunk, then every open
# bug with a fix.
python3 - "$REGISTRY" "$SQL" > "$TMP/builds" <<'PY'
import json, os, sys
reg = json.load(open(sys.argv[1]))
print("trunk - 0 .")
for b in reg["bugs"]:
    if b.get("status") == "open" and b.get("fix"):
        f = b["fix"]
        print(b["name"], os.path.join(sys.argv[2], f["path"]), f.get("strip", 1), f.get("dir", "."))
PY

# Build one CLI into $OUT/$1 (status, build.log, sqlite3).
build() {
    local name=$1 fix=$2 strip=$3 dir=$4
    local dest="$OUT/$name" work="$TMP/work-$name"
    local stamp=trunk
    [ "$fix" != "-" ] && stamp=$(sha256 "$fix")
    if [ -z "${FORCE:-}" ] && [ -x "$dest/sqlite3" ] && [ "$(cat "$dest/status" 2>/dev/null)" = ok ] \
        && [ "$(cat "$dest/fix.sha256" 2>/dev/null)" = "$stamp" ]; then
        echo "$name kept"
        return 0
    fi
    rm -rf "$dest" && mkdir -p "$dest" "$work"
    echo "$stamp" > "$dest/fix.sha256"
    local blog="$dest/build.log"
    if ! (cd "$SRC" && tar cf - -T "$TMP/files") 2> "$blog" | (cd "$work" && tar xf -); then
        echo build_failed > "$dest/status"
        log "$CHECKIN: copying $SRC for the $name CLI failed (see $blog)"
        return 0
    fi
    if [ "$fix" != "-" ]; then
        if ! patch --batch --forward -p"$strip" -d "$work/$dir" -i "$fix" > "$blog" 2>&1; then
            echo patch_stale > "$dest/status"
            log "$CHECKIN: the fix of $name no longer applies (patch -p$strip -d $dir): $(tr '\n' ' ' < "$blog" | cut -c1-300)"
            rm -rf "$work"
            return 0
        fi
        if grep -qiE 'fuzz|offset' "$blog"; then log "$CHECKIN: the fix of $name applied with an offset or fuzz"; fi
    fi
    if (cd "$work" && ./configure && make sqlite3.c shell.c \
            && $CC -O1 $RUNNER_OPTS -o sqlite3 shell.c sqlite3.c -lm -lpthread -ldl) >> "$blog" 2>&1 \
        && "$work/sqlite3" -version >> "$blog" 2>&1; then
        cp "$work/sqlite3" "$dest/sqlite3"
        echo ok > "$dest/status"
        echo "$name ok"
    else
        echo build_failed > "$dest/status"
        log "$CHECKIN: building the $name CLI failed (see $blog)"
    fi
    rm -rf "$work"
}

log "building CLIs for $CHECKIN from $SRC"
PIDS=()
while read -r name fix strip dir; do
    build "$name" "$fix" "$strip" "$dir" &
    PIDS+=($!)
done < "$TMP/builds"
for p in "${PIDS[@]}"; do wait "$p" || true; done

# The check-in's source id (date and hash), and every CLI's status.
if [ -x "$OUT/trunk/sqlite3" ]; then
    "$OUT/trunk/sqlite3" -version > "$OUT/version"
fi
python3 - "$OUT" "$CHECKIN" "$TMP/builds" > "$OUT/status.json.tmp" <<'PY'
import json, os, sys, time
out, checkin, builds = sys.argv[1:4]
clis = {}
for line in open(builds):
    name = line.split()[0]
    try:
        clis[name] = open(os.path.join(out, name, "status")).read().strip()
    except OSError:
        clis[name] = "missing"
version = open(os.path.join(out, "version")).read().strip() if os.path.exists(os.path.join(out, "version")) else None
print(json.dumps({"checkin": checkin, "version": version, "built": int(time.time()), "clis": clis}, indent=2))
PY
mv "$OUT/status.json.tmp" "$OUT/status.json"
log "CLIs for $CHECKIN: $(python3 -c 'import json,sys; print(" ".join(f"{k}={v}" for k,v in json.load(open(sys.argv[1]))["clis"].items()))' "$OUT/status.json")"

# current: the newest check-in (by its source id's date) with a trunk CLI.
NEWEST=$(for d in "$CLIS"/*/; do
    d=${d%/}
    [ -L "$d" ] && continue
    [ -x "$d/trunk/sqlite3" ] && [ -f "$d/version" ] && echo "$(awk '{print $2"T"$3}' "$d/version") $(basename "$d")"
done | sort | tail -1 | awk '{print $2}')
if [ -n "$NEWEST" ]; then
    ln -sfn "$NEWEST" "$CLIS/current"
fi
# Keep the newest CLIS_KEEP check-ins.
KEEP="${CLIS_KEEP:-30}"
for d in "$CLIS"/*/; do
    d=${d%/}
    [ -L "$d" ] || [ ! -f "$d/version" ] || echo "$(awk '{print $2"T"$3}' "$d/version") $d"
done | sort -r | tail -n +"$((KEEP + 1))" | while read -r _ old; do
    [ "$(basename "$old")" = "$NEWEST" ] || { rm -rf "$old"; log "removed old CLIs $old"; }
done
echo "$OUT (current: ${NEWEST:-none})"
