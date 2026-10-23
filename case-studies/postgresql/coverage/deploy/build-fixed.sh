#!/usr/bin/env bash
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

# Build master with the fix of every open bug of known-bugs.json applied, so
# that a run on it stops rediscovering them (PG_TARGET=fixed in runner.env).
#
#     build-fixed.sh [--print] [--ref REF]
#
# - Base: REF, else the commit of PG_BUILDS/current (the master runs use).
# - In a scratch worktree of PG_SRC at the base, each open bug's first
#   available fix source (known-bugs.json: a patch file, the diff of two
#   commits, or `fetch`: a branch fetched from `url` whose diff from its
#   merge base with origin/master is the fix) is applied with
#   `git apply --3way --index`, optional fixes included. A fix that is not
#   available or does not apply is skipped and named in the log.
# - The result is committed (author and date fixed from the base, so the
#   same fixes on the same base give the same commit) on the branch
#   fuzz/fixes-<base12> of PG_SRC, and built by `build-pg.sh --no-fetch
#   --ref <commit>` into PG_BUILDS/<commit12> (a git worktree like any
#   build, so the run's target names the commit).
# - PG_BUILDS/fixed points at the build, PG_BUILDS/<commit12>/fixes.json
#   lists what was applied and skipped. Prints the bin directory.
#
# --print lists the fixes and their availability and touches nothing.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
PRINT=0 REF=""
while [ $# -gt 0 ]; do
    case "$1" in
        --print) PRINT=1 ;;
        --ref) REF="$2"; shift ;;
        *) echo "usage: build-fixed.sh [--print] [--ref REF]" >&2; exit 2 ;;
    esac
    shift
done
say() { echo "$*" >&2; }
REGISTRY="${KNOWN_BUGS:-$HERE/known-bugs.json}"
if [ -n "$REF" ]; then
    BASE=$(git -C "$PG_SRC" rev-parse --verify "$REF^{commit}")
else
    BASE=$(git -C "$PG_BUILDS/current" rev-parse HEAD)
fi
B12=$(echo "$BASE" | cut -c1-12)

# One line per open bug with a fix: id, then the source (file:<path>,
# git:<base>:<head>, fetch:<url>:<ref>, or missing).
SOURCES=$(python3 - "$REGISTRY" "$ROOT" "$PG_SRC" <<'PY'
import json, os, subprocess, sys
reg, root, src = json.load(open(sys.argv[1])), sys.argv[2], sys.argv[3]
def has(c):
    return subprocess.run(["git", "-C", src, "cat-file", "-e", c + "^{commit}"], capture_output=True).returncode == 0
for b in reg["bugs"]:
    f = b.get("fix")
    if b.get("status") != "open" or not f:
        continue
    out = "missing"
    for s in f.get("sources", []):
        if "path" in s and os.path.isfile(os.path.join(root, s["path"])):
            out = "file:" + os.path.join(root, s["path"]); break
        if "git" in s and has(s["git"]["base"]) and has(s["git"]["head"]):
            out = "git:%s:%s" % (s["git"]["base"], s["git"]["head"]); break
        if "fetch" in s:
            out = "fetch:%s:%s" % (s["fetch"]["url"], s["fetch"]["ref"]); break
    print(b["id"], out)
PY
)
if [ "$PRINT" = 1 ]; then
    say "base:   $BASE ($(git -C "$PG_SRC" log -1 --format='%cs %s' "$BASE"))"
    echo "$SOURCES" | sed 's/^/fix:    /' >&2
    exit 0
fi

W="$RUN_ROOT/.fixed-worktree"
git -C "$PG_SRC" worktree remove --force "$W" 2>/dev/null || rm -rf "$W"
git -C "$PG_SRC" worktree prune
git -C "$PG_SRC" worktree add --quiet --detach "$W" "$BASE"
trap 'git -C "$PG_SRC" worktree remove --force "$W" 2>/dev/null || true' EXIT
APPLIED=() SKIPPED=()
DATE=$(git -C "$PG_SRC" log -1 --format=%cI "$BASE")
# Local, never pushed commits of the harness's clone (author ephyra), only
# so that build-pg.sh can build the tree by commit: not signed, since cron
# has no agent to sign with (a signing config made the 2026-10-16 05:30
# refresh fail here).
commit() {
    GIT_AUTHOR_NAME=ephyra GIT_AUTHOR_EMAIL=ephyra@localhost GIT_COMMITTER_NAME=ephyra GIT_COMMITTER_EMAIL=ephyra@localhost \
        GIT_AUTHOR_DATE="$DATE" GIT_COMMITTER_DATE="$DATE" git -C "$W" -c commit.gpgsign=false commit --quiet --no-verify -m "$1"
}
while read -r id src; do
    [ -n "$id" ] || continue
    patch="$RUN_ROOT/.fix-$id.diff"
    case "$src" in
        file:*) cp "${src#file:}" "$patch" ;;
        git:*) IFS=: read -r _ gb gh <<< "$src"; git -C "$PG_SRC" diff "$gb" "$gh" > "$patch" ;;
        fetch:*)
            rest="${src#fetch:}"; url="${rest%:*}"; ref="${rest##*:}"
            if ! git -C "$PG_SRC" fetch --quiet "$url" "+$ref:refs/remotes/fixes/$ref" 2> /dev/null; then
                SKIPPED+=("$id (fetch of $ref failed)"); continue
            fi
            mb=$(git -C "$PG_SRC" merge-base origin/master "refs/remotes/fixes/$ref")
            git -C "$PG_SRC" diff "$mb" "refs/remotes/fixes/$ref" > "$patch" ;;
        *) SKIPPED+=("$id (no fix available)"); continue ;;
    esac
    # Each fix that applies is committed, so a failed one resets to the
    # fixes before it. A fix whose only conflicts are in test suites or meson
    # files (a test list master has since changed) applies without them:
    # the build (make) uses neither.
    if git -C "$W" apply --3way --index "$patch" > "$RUN_ROOT/.fix-$id.log" 2>&1; then
        commit "fix $id"; APPLIED+=("$id")
    elif git -C "$W" reset --quiet --hard \
        && git -C "$W" apply --3way --index --exclude='src/test/*' --exclude='*/meson.build' --exclude='meson.build' "$patch" >> "$RUN_ROOT/.fix-$id.log" 2>&1; then
        commit "fix $id (without its tests)"; APPLIED+=("$id")
        echo "$id: applied without its test and meson files" >> "$RUN_ROOT/.fix-$id.log"
    else
        git -C "$W" reset --quiet --hard
        SKIPPED+=("$id (does not apply: $RUN_ROOT/.fix-$id.log)")
    fi
done <<< "$SOURCES"
[ "${#APPLIED[@]}" -gt 0 ] || { log "no fix applies to $B12; nothing built"; exit 1; }
# One commit on the base: the same fixes on the same base give the same commit.
git -C "$W" reset --quiet --soft "$BASE"
MSG="fuzz fixes on $B12: ${APPLIED[*]}"
commit "$MSG"
FIXED=$(git -C "$W" rev-parse HEAD)
git -C "$PG_SRC" branch --force "fuzz/fixes-$B12" "$FIXED"
git -C "$PG_SRC" worktree remove --force "$W"
trap - EXIT
BIN=$("$HERE/build-pg.sh" --no-fetch --ref "$FIXED" | tail -1)
BUILD="$(cd "$BIN/../.." && pwd -P)"
python3 - "$BUILD/fixes.json" "$BASE" "$FIXED" "${APPLIED[*]}" "$(printf '%s\n' "${SKIPPED[@]+"${SKIPPED[@]}"}")" <<'PY'
import json, sys
p, base, fixed, applied, skipped = sys.argv[1:]
json.dump({"base": base, "commit": fixed, "applied": applied.split(), "skipped": [s for s in skipped.split("\n") if s]},
          open(p, "w"), indent=2)
PY
ln -sfn "$BUILD" "$PG_BUILDS/fixed"
log "fixed build $(echo "$FIXED" | cut -c1-12) on $B12: applied ${APPLIED[*]}${SKIPPED[*]+; skipped ${SKIPPED[*]}}"
echo "$BIN"
