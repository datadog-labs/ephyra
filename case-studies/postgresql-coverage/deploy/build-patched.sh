#!/usr/bin/env bash
# Build the PostgreSQLs pre-triage replays findings with: plain master and,
# for every open bug of known-bugs.json, master with that bug's fix applied.
#
#     build-patched.sh [--print] [--src DIR] [--ref REF] [--master-prefix DIR]
#                      [--out DIR] [--with-optional] [--only BUG]...
#
# - Master: the install of PG_BUILDS/<commit12>/inst (build-pg.sh's; built
#   with `build-pg.sh --no-fetch --ref <commit>` when it is missing), or the
#   install --master-prefix names (used as it is, never written to). REF
#   defaults to the commit of PG_BUILDS/current (the master runs use), else
#   the source tree's origin/master.
# - Each open bug with a fix (`fix` in known-bugs.json) into
#   OUT/<commit12>/<bug>/inst (OUT: --out, else PG_PATCHED, else
#   PG_BUILDS/patched), from the first of its `sources` that is available:
#   a patch file, or the diff of two commits of the source tree (`git`).
#   kind core: `git archive` of the commit, the patch applied, configured
#   as master is (its pg_config --configure, prefix replaced), `make
#   install` and contrib amcheck, btree_gin and btree_gist installed.
#   kind contrib: a copy of the master install with contrib/<dir> (from
#   `git archive`, patched) built and installed into it via PGXS (the
#   copy's pg_config: an install is relocatable). Bugs marked optional
#   (their patch is not saved in the repository) only with --with-optional.
# - OUT/<commit12>/master/inst links to the master install; per build a
#   `status` (ok, patch_missing, patch_stale, build_failed, skipped),
#   `build.log`, `source` (what was applied) and `fix.sha256` (a build of
#   the same patch is kept; FORCE=1 rebuilds); OUT/<commit12>/status.json
#   sums them up with the commit and its date; OUT/current points at the
#   newest commit built. pretriage.py reads OUT/current.
#
# --print prints what would be done (commit, builds, sources and their
# availability, commands) and touches nothing.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

PRINT=0 OPTIONAL=0 SRC="${PG_SRC:-}" REF="" MASTER_PREFIX="" OUT="${PG_PATCHED:-${PG_BUILDS:-$HOME/pg-builds}/patched}"
ONLY=()
while [ $# -gt 0 ]; do
    case "$1" in
        --print) PRINT=1 ;;
        --src) SRC="$2"; shift ;;
        --ref) REF="$2"; shift ;;
        --master-prefix) MASTER_PREFIX="$2"; shift ;;
        --out) OUT="$2"; shift ;;
        --with-optional) OPTIONAL=1 ;;
        --only) ONLY+=("$2"); shift ;;
        *) echo "usage: build-patched.sh [--print] [--src DIR] [--ref REF] [--master-prefix DIR] [--out DIR] [--with-optional] [--only BUG]..." >&2; exit 2 ;;
    esac
    shift
done
REGISTRY="${KNOWN_BUGS:-$HERE/known-bugs.json}"
JOBS="${MAKE_JOBS:-$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)}"
say() { echo "$*" >&2; }

[ -n "$SRC" ] && git -C "$SRC" rev-parse --git-dir > /dev/null 2>&1 || { say "no PostgreSQL git clone at '${SRC}' (--src or PG_SRC)"; exit 1; }
SRC="$(cd "$SRC" && pwd -P)"
if [ -z "$REF" ]; then
    if [ -n "$MASTER_PREFIX" ] && git -C "$MASTER_PREFIX/.." rev-parse HEAD > /dev/null 2>&1 && [ -f "$MASTER_PREFIX/../configure" ]; then
        REF=$(git -C "$MASTER_PREFIX/.." rev-parse HEAD)   # <checkout>/inst, the runner's convention
    elif [ -e "${PG_BUILDS:-/nonexistent}/current" ]; then
        REF=$(git -C "$PG_BUILDS/current" rev-parse HEAD)
    else
        REF=origin/master
    fi
fi
COMMIT=$(git -C "$SRC" rev-parse --verify "$REF^{commit}")
C12=$(echo "$COMMIT" | cut -c1-12)
DATE=$(git -C "$SRC" log -1 --format=%cI "$COMMIT")
DEST="$OUT/$C12"
if [ -n "$MASTER_PREFIX" ]; then
    MASTER_PREFIX="$(cd "$MASTER_PREFIX" && pwd -P)"
else
    MASTER_PREFIX="$PG_BUILDS/$C12/inst"
fi

# name kind dir strip optional source: one line per open bug with a fix;
# source is file:<abs path>, git:<base>:<head>, or missing.
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
python3 - "$REGISTRY" "$SQL" "$OPTIONAL" "${ONLY[@]+"${ONLY[@]}"}" > "$TMP/builds" <<'PY'
import json, os, sys
reg, root, optional, only = json.load(open(sys.argv[1])), sys.argv[2], sys.argv[3] == "1", sys.argv[4:]
for b in reg["bugs"]:
    f = b.get("fix")
    if b.get("status") != "open" or not f or (only and b["id"] not in only):
        continue
    src, strip = "missing", 1
    for s in f.get("sources", []):
        if "path" in s and os.path.isfile(os.path.join(root, s["path"])):
            src, strip = "file:" + os.path.join(root, s["path"]), s.get("strip", 1)
            break
        if "git" in s:
            src = "git:%s:%s" % (s["git"]["base"], s["git"]["head"])
            break
    skip = f.get("optional") and not optional
    print(b["id"], f.get("kind", "core"), f.get("dir", "."), strip, "skip" if skip else "build", src)
PY

configure_args() {
    # Master's configure line, its --prefix replaced by $1.
    "$MASTER_PREFIX/bin/pg_config" --configure | python3 -c '
import shlex, sys
args = [a for a in shlex.split(sys.stdin.read()) if not a.startswith("--prefix=")]
print("\0".join(["--prefix=" + sys.argv[1]] + args), end="")' "$1"
}

# The patch of a source into $2 (0: available).
get_patch() {
    case "$1" in
        file:*) cp "${1#file:}" "$2" ;;
        git:*)
            local spec=${1#git:}
            GIT_NO_LAZY_FETCH=1 git -C "$SRC" diff "${spec%%:*}" "${spec##*:}" > "$2" 2> /dev/null && [ -s "$2" ] ;;
        *) return 1 ;;
    esac
}

sha() { if command -v sha256sum > /dev/null; then sha256sum "$1" | cut -c1-64; else shasum -a 256 "$1" | cut -c1-64; fi; }

if [ "$PRINT" = 1 ]; then
    say "source:  $SRC"
    say "commit:  $COMMIT ($DATE)"
    if [ -f "$MASTER_PREFIX/bin/postgres" ]; then
        say "master:  $MASTER_PREFIX (exists; linked as $DEST/master/inst)"
    else
        say "master:  $MASTER_PREFIX (missing: would run build-pg.sh --no-fetch --ref $COMMIT)"
    fi
    while read -r name kind dir strip mode src; do
        avail=yes
        if [ "$src" = missing ]; then avail="no (see the registry's obtain)"
        elif [[ $src == git:* ]]; then get_patch "$src" "$TMP/p" || avail="no: the commits are not in $SRC (see obtain)"; fi
        say "$name: $kind${dir:+ $dir}, $mode, source $src, available: $avail"
        [ "$mode" = skip ] && { say "    skipped (optional; --with-optional builds it)"; continue; }
        if [ "$kind" = contrib ]; then
            say "    cp -R $MASTER_PREFIX $DEST/$name/inst; git archive $C12 $dir | patch -p$strip; make -C $dir USE_PGXS=1 PG_CONFIG=$DEST/$name/inst/bin/pg_config install"
        else
            say "    git archive $C12 into $DEST/$name/src; patch -p$strip; ./configure --prefix=$DEST/$name/inst <master's flags>; make -j$JOBS install; contrib amcheck btree_gin btree_gist"
        fi
    done < "$TMP/builds"
    echo "$DEST"
    exit 0
fi

if [ ! -x "$MASTER_PREFIX/bin/postgres" ]; then
    log "no master install at $MASTER_PREFIX: building it with build-pg.sh"
    "$HERE/build-pg.sh" --no-fetch --ref "$COMMIT" > /dev/null
fi
mkdir -p "$DEST/master"
ln -sfn "$MASTER_PREFIX" "$DEST/master/inst"
echo ok > "$DEST/master/status"
echo "master: $MASTER_PREFIX" > "$DEST/master/source"

build_one() {
    local name=$1 kind=$2 dir=$3 strip=$4 mode=$5 src=$6
    local b="$DEST/$name"
    mkdir -p "$b"
    local blog="$b/build.log"
    if [ "$mode" = skip ]; then
        if [ "$(cat "$b/status" 2>/dev/null)" != ok ]; then echo skipped > "$b/status"; fi
        echo "$name: $(cat "$b/status") (optional)"
        return 0
    fi
    if ! get_patch "$src" "$TMP/$name.patch"; then
        echo patch_missing > "$b/status"
        echo "$src" > "$b/source"
        log "$C12: no patch for $name ($src; see known-bugs.json's obtain)"
        echo "$name: patch_missing"
        return 0
    fi
    local stamp
    stamp=$(sha "$TMP/$name.patch")
    if [ -z "${FORCE:-}" ] && [ "$(cat "$b/status" 2>/dev/null)" = ok ] && [ "$(cat "$b/fix.sha256" 2>/dev/null)" = "$stamp" ] \
        && [ -x "$b/inst/bin/postgres" ]; then
        echo "$name: kept"
        return 0
    fi
    rm -rf "$b/inst" "$b/src"
    echo "$stamp" > "$b/fix.sha256"
    echo "$src" > "$b/source"
    cp "$TMP/$name.patch" "$b/fix.patch"
    mkdir -p "$b/src"
    local paths=()
    [ "$kind" = contrib ] && paths=("$dir")
    if ! git -C "$SRC" archive "$COMMIT" "${paths[@]+"${paths[@]}"}" 2> "$blog" | tar xf - -C "$b/src" 2>> "$blog"; then
        echo build_failed > "$b/status"; log "$C12: git archive for $name failed ($blog)"; echo "$name: build_failed"; return 0
    fi
    if ! patch --batch --forward -p"$strip" -d "$b/src" -i "$b/fix.patch" >> "$blog" 2>&1; then
        echo patch_stale > "$b/status"
        log "$C12: the fix of $name no longer applies: $(tail -3 "$blog" | tr '\n' ' ' | cut -c1-300)"
        rm -rf "$b/src"
        echo "$name: patch_stale"
        return 0
    fi
    grep -qiE 'fuzz|offset' "$blog" && log "$C12: the fix of $name applied with an offset or fuzz"
    local ok=1
    if [ "$kind" = contrib ]; then
        cp -R "$MASTER_PREFIX" "$b/inst" \
            && make -C "$b/src/$dir" USE_PGXS=1 PG_CONFIG="$b/inst/bin/pg_config" -j"$JOBS" install >> "$blog" 2>&1 || ok=0
    else
        local args=()
        while IFS= read -r -d '' a; do args+=("$a"); done < <(configure_args "$b/inst"; printf '\0')
        (cd "$b/src" && ./configure "${args[@]}" && make -j"$JOBS" && make install \
            && for m in amcheck btree_gin btree_gist; do make -C "contrib/$m" install || exit 1; done) >> "$blog" 2>&1 || ok=0
    fi
    if [ "$ok" = 1 ] && "$b/inst/bin/postgres" -V >> "$blog" 2>&1; then
        echo ok > "$b/status"
        rm -rf "$b/src"
        echo "$name: ok"
    else
        echo build_failed > "$b/status"
        log "$C12: building $name failed ($blog)"
        echo "$name: build_failed"
    fi
}

log "building patched PostgreSQLs for $C12 into $DEST"
while read -r name kind dir strip mode src; do
    build_one "$name" "$kind" "$dir" "$strip" "$mode" "$src"
done < "$TMP/builds"

python3 - "$DEST" "$COMMIT" "$DATE" "$MASTER_PREFIX" > "$DEST/status.json.tmp" <<'PY'
import json, os, sys, time
dest, commit, date, master = sys.argv[1:5]
builds = {}
for name in sorted(os.listdir(dest)):
    p = os.path.join(dest, name, "status")
    if os.path.isfile(p):
        src = os.path.join(dest, name, "source")
        builds[name] = {"status": open(p).read().strip(), "source": open(src).read().strip() if os.path.isfile(src) else None}
print(json.dumps({"commit": commit, "commit_date": date, "master": master, "built": int(time.time()), "builds": builds}, indent=2))
PY
mv "$DEST/status.json.tmp" "$DEST/status.json"
log "patched builds for $C12: $(python3 -c 'import json,sys; print(" ".join("%s=%s" % (k, v["status"]) for k, v in json.load(open(sys.argv[1]))["builds"].items()))' "$DEST/status.json")"

# current: the newest commit (by date) with a master build.
NEWEST=$(for s in "$OUT"/*/status.json; do
    [ -f "$s" ] && python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); print(d["commit_date"], d["commit"][:12])' "$s"
done | sort | tail -1 | awk '{print $2}')
[ -n "$NEWEST" ] && ln -sfn "$NEWEST" "$OUT/current"
echo "$DEST"
