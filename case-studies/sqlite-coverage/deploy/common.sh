# Shared by the deploy scripts: settings, paths, and a log line.
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
COV="$(cd "$HERE/.." && pwd)"
SQL="$(cd "$COV/.." && pwd)"
VENDOR="$SQL/vendor/libsqlite3-sys-trunk"
# shellcheck disable=SC1090
source "${RUNNER_ENV:-$HERE/runner.env}"
# cron runs with a bare PATH.
[ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"
export PATH="$HOME/.cargo/bin:/usr/local/bin:/usr/bin:/bin:$PATH"
mkdir -p "$RUN_ROOT"
if [ -n "${SQLITE_TMPDIR:-}" ]; then export SQLITE_TMPDIR; fi

log() {
    echo "$(date -u +%Y-%m-%dT%H:%M:%SZ) $(basename "$0"): $*" >> "$RUN_ROOT/deploy.log"
}

# The 12-character check-in of the SQLite whose sqlite3.h is $1.
checkin() {
    grep -m1 'define SQLITE_SOURCE_ID' "$1" | awk '{print $5}' | cut -c1-12
}

# The files of a SQLite source tree $1 (its manifest's, and the manifest
# itself), one per line: what a fresh copy of it takes, never a configured
# tree's build products (configure writes the tree's absolute path into its
# Makefile, so a copy is configured inside itself).
tree_files() {
    grep '^F ' "$1/manifest" | awk '{print $2}'
    echo manifest
    echo manifest.uuid
    if [ -f "$1/manifest.tags" ]; then echo manifest.tags; fi
}

# The runner's SQLite options (vendor/libsqlite3-sys-trunk's build.rs,
# bundled), with which the pre-triage CLIs are compiled.
# shellcheck disable=SC2034
RUNNER_OPTS="-DSQLITE_DEFAULT_FOREIGN_KEYS=1 -DSQLITE_ENABLE_API_ARMOR -DSQLITE_ENABLE_COLUMN_METADATA \
-DSQLITE_ENABLE_DBSTAT_VTAB -DSQLITE_ENABLE_FTS3 -DSQLITE_ENABLE_FTS3_PARENTHESIS -DSQLITE_ENABLE_FTS5 \
-DSQLITE_ENABLE_JSON1 -DSQLITE_ENABLE_LOAD_EXTENSION=1 -DSQLITE_ENABLE_MEMORY_MANAGEMENT -DSQLITE_ENABLE_RTREE -DSQLITE_ENABLE_GEOPOLY \
-DSQLITE_ENABLE_STAT4 -DSQLITE_SOUNDEX -DSQLITE_THREADSAFE=1 -DSQLITE_USE_URI -DHAVE_USLEEP=1 -DHAVE_ISNAN \
-DHAVE_LOCALTIME_R"

sha256() { if command -v sha256sum > /dev/null; then sha256sum "$1" | cut -c1-64; else shasum -a 256 "$1" | cut -c1-64; fi; }
