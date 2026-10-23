#!/usr/bin/env bash
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

# build.sh <src-dir> <build-dir> [coverage]
# Builds a stage1 rustc (debug assertions, CI LLVM with assertions, llvm + cranelift
# backends) plus its std from <src-dir> (a git worktree of rust-lang/rust, possibly
# with uncommitted patches), entirely inside <build-dir>. Incremental, re-runnable.
# With `coverage`, the stage1 compiler crates are built with -Cinstrument-coverage
# (by the stage0 compiler, via RUSTFLAGS_BOOTSTRAP) and <build-dir>/coverage.json is written.
set -euo pipefail
SRC=$(realpath "$1"); OUT=$(mkdir -p "$2" && realpath "$2"); MODE=${3:-}
HERE=$(cd "$(dirname "$0")" && pwd)
WT="$OUT/src"
mkdir -p "$WT"
# Private copy of the tree (keeps the .git file so bootstrap's git queries work; it only
# reads history to pick the CI LLVM artifact). Submodule dirs we populate are excluded
# from --delete. mtimes are preserved, so unchanged files don't trigger rebuilds.
rsync -a --delete --exclude=/library/backtrace/ --exclude=/src/llvm-project/ \
  --exclude=/default*.profraw "$SRC"/ "$WT"/
# std's backtrace submodule: fetch the pinned commit directly (no `git submodule`).
BT=$(git -C "$SRC" ls-tree HEAD library/backtrace | awk '{print $3}')
if [ "$(git -C "$WT/library/backtrace" rev-parse HEAD 2>/dev/null || true)" != "$BT" ]; then
  rm -rf "$WT/library/backtrace"; mkdir -p "$WT/library/backtrace"
  git -C "$WT/library/backtrace" init -q
  git -C "$WT/library/backtrace" fetch -q --depth 1 https://github.com/rust-lang/backtrace-rs.git "$BT"
  git -C "$WT/library/backtrace" checkout -q FETCH_HEAD
fi
# compiler-rt sources (optimized compiler-builtins): sparse fetch of the pinned llvm-project.
LP=$(git -C "$SRC" ls-tree HEAD src/llvm-project | awk '{print $3}')
if [ "$(git -C "$WT/src/llvm-project" rev-parse HEAD 2>/dev/null || true)" != "$LP" ]; then
  rm -rf "$WT/src/llvm-project"; mkdir -p "$WT/src/llvm-project"
  git -C "$WT/src/llvm-project" init -q
  git -C "$WT/src/llvm-project" sparse-checkout set --no-cone /compiler-rt/ /cmake/ /llvm/cmake/
  git -C "$WT/src/llvm-project" fetch -q --depth 1 --filter=blob:none https://github.com/rust-lang/llvm-project.git "$LP"
  git -C "$WT/src/llvm-project" checkout -q FETCH_HEAD
fi
cat > "$OUT/bootstrap.toml" <<EOF
change-id = "ignore"
[build]
build-dir = "$OUT/build"
submodules = false
extended = false
docs = false
compiler-docs = false
locked-deps = true
[llvm]
download-ci-llvm = true
assertions = true
[rust]
channel = "nightly"
debug-assertions = true
debug-assertions-std = false
overflow-checks = true
debuginfo-level = "line-tables-only"
backtrace-on-ice = true
codegen-backends = ["llvm", "cranelift"]
deny-warnings = false
incremental = false
lto = "off"
download-rustc = false
EOF
# rustlantis (custom-MIR program generator, run.py uses it for a share of the cases): pinned
# commit, built with the host's stable cargo (it uses #![feature], hence RUSTC_BOOTSTRAP).
# rustlantis-noref.patch: raw pointers instead of references (its reference tracking makes most
# programs Stacked/Tree Borrows UB), and addr_of! only of initialized places (RFC 3943, README). Optional: if this fails, run.py only uses gen.py.
RL_COMMIT=2be7c5a6763831dc0f908f0e05723ebbbc2a62cc
RL_STAMP="$RL_COMMIT $(sha1sum < "$HERE/rustlantis-noref.patch" | cut -c1-12)"
if [ ! -x "$OUT/rustlantis/generate" ] || [ "$(cat "$OUT/rustlantis/stamp" 2>/dev/null)" != "$RL_STAMP" ]; then
  # separate bash so that set -e holds inside (it is ignored in a subshell on the left of ||)
  if ! bash -euo pipefail -s "$OUT" "$HERE" "$RL_COMMIT" "$RL_STAMP" <<'RLEOF'
OUT=$1; HERE=$2; RL_COMMIT=$3; RL_STAMP=$4
rm -rf "$OUT/rustlantis"
mkdir -p "$OUT/rustlantis-src"; cd "$OUT/rustlantis-src"
[ -d .git ] || git init -q
git fetch -q --depth 1 https://github.com/cbeuw/rustlantis.git "$RL_COMMIT"
git checkout -q -f "$RL_COMMIT"
git apply "$HERE/rustlantis-noref.patch"
env -u RUSTFLAGS RUSTC_BOOTSTRAP=1 CARGO_TARGET_DIR="$OUT/rustlantis-target" cargo build -q --release --bin generate
mkdir -p "$OUT/rustlantis"; cp "$OUT/rustlantis-target/release/generate" "$OUT/rustlantis/generate"
echo "$RL_STAMP" > "$OUT/rustlantis/stamp"
RLEOF
  then
    echo "build.sh: rustlantis build failed; run.py will use gen.py only" >&2
  fi
fi
cd "$WT"
if [ "$MODE" = coverage ]; then
  export RUSTFLAGS_BOOTSTRAP="-Cinstrument-coverage"
  mkdir -p "$OUT/build-profraw"
  export LLVM_PROFILE_FILE="$OUT/build-profraw/%m.profraw"
fi
python3 x.py build --config "$OUT/bootstrap.toml" --stage 1 library
H=$OUT/build/x86_64-unknown-linux-gnu
ln -sfn "$H/stage1/bin/rustc" "$OUT/rustc"
"$OUT/rustc" -vV
if [ "$MODE" = coverage ]; then
  rm -rf "$OUT/build-profraw"
  python3 - "$OUT" "$H" <<'EOF'
import glob, json, os, sys
out, h = sys.argv[1], sys.argv[2]
objs = [os.path.realpath(f"{h}/stage1/bin/rustc")]
objs += sorted(glob.glob(f"{h}/stage1/lib/librustc_driver-*.so"))
objs += sorted(glob.glob(f"{h}/stage1/lib/rustlib/x86_64-unknown-linux-gnu/codegen-backends/*.so"))
json.dump({"objects": objs,
           "profdata": f"{h}/ci-llvm/bin/llvm-profdata",
           "cov": f"{h}/ci-llvm/bin/llvm-cov"}, open(f"{out}/coverage.json", "w"), indent=1)
print(open(f"{out}/coverage.json").read())
EOF
fi
