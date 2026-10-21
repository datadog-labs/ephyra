# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

# Shared by the deploy scripts: settings, paths, and a log line.
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
COV="$(cd "$HERE/.." && pwd)"
SQL="$(cd "$COV/.." && pwd)"
EPG="$SQL/ephyra-postgresql"
# shellcheck disable=SC1090
source "${RUNNER_ENV:-$HERE/runner.env}"
# cron runs with a bare PATH.
# shellcheck disable=SC1091
[ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"
export PATH="$HOME/.cargo/bin:/usr/local/bin:/usr/bin:/bin:$PATH"
# The runner's test hook must never reach a real run.
unset EPHYRA_RUNNER_INJECT
mkdir -p "$RUN_ROOT"

log() {
    echo "$(date -u +%Y-%m-%dT%H:%M:%SZ) $(basename "$0"): $*" >> "$RUN_ROOT/deploy.log"
}

# The 12-character commit of the checkout whose inst/bin is $1 (the runner's
# own convention for its target tag: <checkout>/inst/bin).
pg_commit() {
    git -C "$1/../.." rev-parse HEAD 2>/dev/null | cut -c1-12
}

# The binaries a new run uses: PG_BIN when set, else with PG_TARGET=fixed
# the build PG_BUILDS/fixed points at (build-fixed.sh's: master with the
# known bugs' fixes), else the build PG_BUILDS/current points at
# (refresh-master.sh moves both), resolved so that moving a link never
# changes a running run's binaries.
pg_bin() {
    if [ -n "${PG_BIN:-}" ]; then
        (cd "$PG_BIN" && pwd -P)
    elif [ "${PG_TARGET:-master}" = fixed ]; then
        (cd "$PG_BUILDS/fixed/inst/bin" && pwd -P)
    else
        (cd "$PG_BUILDS/current/inst/bin" && pwd -P)
    fi
}
