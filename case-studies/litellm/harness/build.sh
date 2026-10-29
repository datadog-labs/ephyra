#!/usr/bin/env bash
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

# build.sh <src-dir> <build-dir> [coverage]
# Python target: a venv with litellm's locked deps (the package itself is imported
# through PYTHONPATH from a snapshot of <src-dir> in <build-dir>/src; coverage paths
# map back to the tree by suffix), plus the Rust ephyra driver (bin `search`).
# Incremental and re-runnable.
set -euo pipefail
SRC=$(cd "$1" && pwd)
mkdir -p "$2"
B=$(cd "$2" && pwd)
MODE=${3:-}
H=$(cd "$(dirname "$0")" && pwd)

export UV_CACHE_DIR=/agent-run/work/.cache/uv
unset UV_INDEX PIP_EXTRA_INDEX_URL || true
export UV_DEFAULT_INDEX=https://pypi.org/simple
export RUSTUP_TOOLCHAIN=stable
export HOME=${HOME:-/tmp}

# 1. deps (src untouched: --no-install-project)
( cd "$SRC" && UV_PROJECT_ENVIRONMENT="$B/venv" uv sync --frozen --extra proxy --no-install-project --python 3.12 -q )
"$B/venv/bin/python" -c 'import coverage, httpx' >/dev/null

# 2. the ephyra driver
( cd "$H/driver" && CARGO_TARGET_DIR="$B/cargo-target" cargo build --release -q )
cp "$B/cargo-target/release/search" "$B/search"

# 3. snapshot of the source. Workers must not import from <src-dir>: the controller
# rebuilds by `git reset --hard` + re-applying patches/ in that tree, so a search
# worker or repro.sh that started inside the window imported unpatched modules
# (seen as already-fixed bugs coming back, e.g. pre-012 caching_handler). Swap the
# `src` symlink atomically; keep the two previous snapshots for running workers.
SNAP="src.$(date +%s%N)"
rsync -a --exclude=.git --exclude=__pycache__ --exclude=node_modules \
  --exclude=/tests --exclude=/ui --exclude=/cookbook "$SRC/" "$B/$SNAP/"
ln -sfn "$SNAP" "$B/src.next" && mv -T "$B/src.next" "$B/src"
ls -1d "$B"/src.[0-9]* | sort | head -n -3 | xargs -r rm -rf

# 4. worker + environment
mkdir -p "$B/harness"
cp "$H/worker.py" "$B/harness/worker.py"
cat > "$B/env.sh" <<EOF
export LITELLM_SRC="$B/src"
export LITELLM_PY="$B/venv/bin/python"
export PYTHONPATH="$B/src"
export PYTHONPYCACHEPREFIX="$B/pyc"
export LITELLM_LOCAL_MODEL_COST_MAP=True
export LITELLM_TELEMETRY=False
export COVERAGE_MODE="$MODE"
export NO_PROXY='*'
EOF

# smoke: litellm imports from the snapshot
( source "$B/env.sh" && "$B/venv/bin/python" -c 'import litellm,sys; assert litellm.__file__.startswith(sys.argv[1]), litellm.__file__' "$B/src/" )

if [ "$MODE" = coverage ]; then
  printf '{"kind": "coveragepy", "coverage": "%s"}\n' "$B/venv/bin/coverage" > "$B/coverage.json"
fi
echo "build ok: $B"
