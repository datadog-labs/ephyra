#!/usr/bin/env bash
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

# Build trunk with the fix of every open bug of known-bugs.json applied, so
# that a run on it stops rediscovering them (SQLITE_TARGET=fixed in
# runner.env; refresh-trunk.sh vendors the result).
#
#     build-fixed.sh [--print] [--out DIR] [SRC_TREE]
#
# - Base: a SQLite source tree (SRC_TREE, e.g. refresh-trunk.sh's extracted
#   and configured trunk, or else the vendored check-in's tarball,
#   downloaded). Its manifest's files are copied into a fresh tree (never
#   the base's build products: configure writes the tree's absolute path
#   into its Makefile, so the copy is configured inside itself).
# - Every open bug with a fix or a fix-trunk.diff in its folder (a bug
#   without a registry fix, `fix: null`, may have one), in the registry's
#   order: its folder's fix-trunk.diff when there is one (a fix rebased
#   onto trunk), else its fix.path, applied with `patch -p<strip> -d <dir>` (the registry's
#   options, else -p1 or -p0 from the top, whichever applies) after a dry
#   run on the tree as patched so far. A fix-trunk.diff that does not apply
#   falls back to fix.path; a fix that applies in no way is skipped and
#   named in the log (two fixes touching the same lines: the later one).
# - The patched tree's amalgamation (`make sqlite3.c shell.c`; its
#   SQLITE_SOURCE_ID keeps the check-in and ends in "alt1", SQLite's mark
#   of a modified tree) and a CLI compiled from it with the runner's own
#   options go to DIR (default RUN_ROOT/fixed/<check-in>/: sqlite3.c,
#   sqlite3.h, sqlite3ext.h, sqlite3, fixes.json, build.log, patches/); the
#   CLI also to RUN_ROOT/clis/<check-in>/fixed/ (sqlite3, fixes.json,
#   status, build.log), where pre-triage replays a finding on the run's own
#   SQLite. Keeps the newest FIXED_KEEP (3) builds of RUN_ROOT/fixed/.
# - fixes.json: the check-in, the source id, the applied fixes (`applied`,
#   bug names) and each one's patch file, options and sha256 (`patches`),
#   the skipped ones and why (`skipped`), and sqlite3.c's sha256, by which
#   start-run.sh knows the vendored amalgamation is this build.
# Prints DIR. Fails (and builds nothing) when no fix applies.
#
# --print lists each fix's patch file and, with SRC_TREE, whether it
# applies to that tree on its own; touches nothing.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
PRINT=0 DEST="" SRC_ARG=""
while [ $# -gt 0 ]; do
    case "$1" in
        --print) PRINT=1 ;;
        --out) DEST="$2"; shift ;;
        -*) echo "usage: build-fixed.sh [--print] [--out DIR] [SRC_TREE]" >&2; exit 2 ;;
        *) SRC_ARG="$1" ;;
    esac
    shift
done
say() { echo "$*" >&2; }
REGISTRY="${KNOWN_BUGS:-$HERE/known-bugs.json}"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

# One line per open bug with a fix: name, strip, dir, then the patch files
# to try in order (fix-trunk.diff beside the fix when present, the fix), or
# "missing" when neither exists.
python3 - "$REGISTRY" "$ROOT" > "$TMP/fixes" <<'PY'
import json, os, sys
reg, root = json.load(open(sys.argv[1])), sys.argv[2]
for b in reg["bugs"]:
    f = b.get("fix") or {}
    folder = os.path.join(root, b["folder"]) if b.get("folder") else None
    if b.get("status") != "open":
        continue
    trunk = os.path.join(folder, "fix-trunk.diff") if folder else None
    if not f and not (trunk and os.path.isfile(trunk)):
        continue
    files = [p for p in (trunk, os.path.join(root, f["path"]) if f else None) if p and os.path.isfile(p)]
    files = list(dict.fromkeys(files))
    print(b["name"], f.get("strip", 1), f.get("dir", "."), " ".join(files) or "missing")
PY

if [ -n "$SRC_ARG" ]; then
    SRC="$(cd "$SRC_ARG" && pwd -P)"
elif [ "$PRINT" = 0 ]; then
    WANT=trunk
    [ -f "$VENDOR/sqlite3/sqlite3.h" ] && WANT=$(checkin "$VENDOR/sqlite3/sqlite3.h")
    curl -sfL "https://sqlite.org/src/tarball/$WANT/sqlite.tar.gz" -o "$TMP/sqlite.tar.gz"
    mkdir "$TMP/download"
    tar xzf "$TMP/sqlite.tar.gz" -C "$TMP/download"
    SRC="$TMP/download/sqlite"
else
    SRC=""
fi
if [ -n "$SRC" ]; then
    [ -f "$SRC/manifest" ] && [ -f "$SRC/manifest.uuid" ] || { say "no manifest in $SRC: not a SQLite source tree"; exit 1; }
fi

# Apply patch $1 to tree $2 with the registry's options $3 (strip) and $4
# (dir), else -p1 or -p0 from the top: the first a dry run accepts. Prints
# the options used ("<strip> <dir>"); output of the apply in $5.
apply_one() {
    local p=$1 tree=$2 strip=$3 dir=$4 out=$5 s d log
    for opt in "$strip $dir" "1 ." "0 ."; do
        read -r s d <<< "$opt"
        [ -d "$tree/$d" ] || continue
        # The registry options' dry run is the one the log keeps on failure.
        log="$out"; [ "$opt" = "$strip $dir" ] || log="$out.alt"
        if patch --batch --forward --dry-run -p"$s" -d "$tree/$d" -i "$p" > "$log" 2>&1; then
            patch --batch --forward -p"$s" -d "$tree/$d" -i "$p" > "$out" 2>&1 || return 1
            rm -f "$out.alt"
            echo "$s $d"
            return 0
        fi
    done
    rm -f "$out.alt"
    return 1
}

# Whether patch $1 is already in tree $2 (it applies reversed, with the
# registry's options $3 and $4, else -p1 or -p0 from the top): trunk has the
# fix, and the bug is for retiring.
in_tree() {
    local p=$1 tree=$2 strip=$3 dir=$4 s d
    for opt in "$strip $dir" "1 ." "0 ."; do
        read -r s d <<< "$opt"
        [ -d "$tree/$d" ] || continue
        patch --batch --reverse --dry-run -p"$s" -d "$tree/$d" -i "$p" > /dev/null 2>&1 && return 0
    done
    return 1
}

if [ "$PRINT" = 1 ]; then
    [ -n "$SRC" ] && say "base:   $(cut -c1-12 "$SRC/manifest.uuid") ($SRC)"
    while read -r name strip dir files; do
        if [ "$files" = missing ]; then say "fix:    $name: no patch file"; continue; fi
        first=${files%% *}
        line="fix:    $name: ${first#"$ROOT"/} (-p$strip -d $dir)"
        if [ -n "$SRC" ]; then
            ok=""
            for f in $files; do
                rm -rf "$TMP/one" && mkdir "$TMP/one"
                (cd "$SRC" && tree_files "$SRC" | tar cf - -T -) | (cd "$TMP/one" && tar xf -)
                if used=$(apply_one "$f" "$TMP/one" "$strip" "$dir" "$TMP/one.log"); then ok="${f#"$ROOT"/} applies (-p${used% *} -d ${used#* })"; break; fi
                if in_tree "$f" "$TMP/one" "$strip" "$dir"; then ok="already in the tree (${f#"$ROOT"/} applies reversed): fixed upstream?"; break; fi
            done
            line="$line: ${ok:-does not apply}"
        fi
        say "$line"
    done < "$TMP/fixes"
    exit 0
fi

CHECKIN=$(cut -c1-12 "$SRC/manifest.uuid")
OUT="${DEST:-$RUN_ROOT/fixed/$CHECKIN}"
W="$TMP/tree"
mkdir -p "$W" "$TMP/patches"
(cd "$SRC" && tree_files "$SRC" | tar cf - -T -) | (cd "$W" && tar xf -)

APPLIED=() SKIPPED=()
: > "$TMP/applied.tsv"
while read -r name strip dir files; do
    [ -n "$name" ] || continue
    if [ "$files" = missing ]; then
        SKIPPED+=("$name: no patch file")
        continue
    fi
    done_one=""
    for f in $files; do
        if used=$(apply_one "$f" "$W" "$strip" "$dir" "$TMP/patches/$name.log"); then
            fuzz=false
            grep -qiE 'fuzz|offset' "$TMP/patches/$name.log" && fuzz=true
            printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$name" "${f#"$ROOT"/}" "${used% *}" "${used#* }" "$(sha256 "$f")" "$fuzz" >> "$TMP/applied.tsv"
            cp "$f" "$TMP/patches/$name.diff"
            APPLIED+=("$name")
            done_one=1
            break
        fi
    done
    if [ -z "$done_one" ]; then
        for f in $files; do
            if in_tree "$f" "$W" "$strip" "$dir"; then done_one="${f#"$ROOT"/}"; break; fi
        done
    fi
    if [ -n "$done_one" ] && [ "$done_one" != 1 ]; then
        SKIPPED+=("$name: already in the tree ($done_one applies reversed): fixed upstream?")
    elif [ -z "$done_one" ]; then
        SKIPPED+=("$name: does not apply (${files//$ROOT\//}): $(tr '\n' ' ' < "$TMP/patches/$name.log" | cut -c1-200)")
    fi
done < "$TMP/fixes"
if [ "${#APPLIED[@]}" -eq 0 ]; then
    log "no fix applies to $CHECKIN; nothing built"
    say "no fix applies to $CHECKIN"
    exit 1
fi

BLOG="$TMP/build.log"
CC="${CC:-cc}"
# shellcheck disable=SC2086
if ! (cd "$W" && ./configure && make sqlite3.c shell.c \
        && $CC -O1 $RUNNER_OPTS -o sqlite3 shell.c sqlite3.c -lm -lpthread -ldl \
        && ./sqlite3 -version) > "$BLOG" 2>&1; then
    log "building trunk $CHECKIN with the fixes failed: $(tail -5 "$BLOG" | tr '\n' ' ' | cut -c1-300)"
    say "build failed:"; tail -20 "$BLOG" >&2
    exit 1
fi

rm -rf "$OUT.tmp" && mkdir -p "$OUT.tmp"
cp "$W/sqlite3.c" "$W/sqlite3.h" "$W/src/sqlite3ext.h" "$W/sqlite3" "$BLOG" "$OUT.tmp/"
cp -R "$TMP/patches" "$OUT.tmp/patches"
python3 - "$OUT.tmp" "$CHECKIN" "$(cat "$W/manifest.uuid")" "$TMP/applied.tsv" "$(printf '%s\n' "${SKIPPED[@]+"${SKIPPED[@]}"}")" "$(sha256 "$W/sqlite3.c")" <<'PY'
import json, re, sys, time
out, checkin, base, applied_tsv, skipped, c_sha = sys.argv[1:]
patches = {}
for line in open(applied_tsv):
    name, path, strip, d, sha, fuzz = line.rstrip("\n").split("\t")
    patches[name] = {"path": path, "strip": int(strip), "dir": d, "sha256": sha, "fuzz_or_offset": fuzz == "true"}
src = next(l for l in open(out + "/sqlite3.h") if "define SQLITE_SOURCE_ID" in l)
json.dump({"checkin": checkin, "base": base, "source_id": re.search(r'"(.*)"', src).group(1), "built": int(time.time()),
           "applied": list(patches), "patches": patches,
           "skipped": [dict(zip(("bug", "reason"), s.split(": ", 1))) for s in skipped.split("\n") if s],
           "sqlite3_c_sha256": c_sha},
          open(out + "/fixes.json", "w"), indent=2)
PY
rm -rf "$OUT" && mv "$OUT.tmp" "$OUT"

# The CLI where pre-triage looks for the run's own SQLite.
CLI="$RUN_ROOT/clis/$CHECKIN/fixed"
rm -rf "$CLI" && mkdir -p "$CLI"
cp "$OUT/sqlite3" "$OUT/fixes.json" "$OUT/build.log" "$CLI/"
echo ok > "$CLI/status"

# Keep the newest FIXED_KEEP builds (the default DIR's siblings).
if [ -z "$DEST" ]; then
    # shellcheck disable=SC2012
    ls -1dt "$RUN_ROOT/fixed"/*/ 2>/dev/null | sed 's:/$::' | tail -n +"$((${FIXED_KEEP:-3} + 1))" | while read -r old; do
        rm -rf "$old"
        log "removed old fixed build $old"
    done
fi
log "fixed build of $CHECKIN in $OUT: applied ${APPLIED[*]}${SKIPPED[*]+; skipped: $(printf '%s; ' "${SKIPPED[@]}")}"
say "applied: ${APPLIED[*]}"
for s in "${SKIPPED[@]+"${SKIPPED[@]}"}"; do say "skipped: $s"; done
echo "$OUT"
