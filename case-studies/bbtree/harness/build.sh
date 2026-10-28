#!/usr/bin/env bash
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

# build.sh <src-dir> <build-dir> [coverage]
# Builds the bbz harness binary against the bbtree tree in <src-dir> (the workspace with
# bbtree/, bbtree-mmap/, bbtree-store/), with --cfg fuzzing so bbtree's own invariant checks
# (check_invariants, check_prefix_lens) are compiled in, and debug assertions on.
# Base mode also tries bin/bbz-asan (nightly, AddressSanitizer); without nightly it warns and goes on.
# With "coverage": instrumented for LLVM source-based coverage, plus <build-dir>/coverage.json.
# Everything (crate copy, cargo target dirs, binaries) goes into <build-dir>.
set -euo pipefail
SRC="$(cd "$1" && pwd)"
mkdir -p "$2"
BUILD="$(cd "$2" && pwd)"
HERE="$(cd "$(dirname "$0")" && pwd)"
MODE="${3:-}"

export CARGO_HOME="${CARGO_HOME:-/agent-run/work/.cargo}"
[ -d "$CARGO_HOME" ] || unset CARGO_HOME
EPHYRA="${EPHYRA:-$(cd "$(dirname "$0")/../../.." && pwd)}"
[ -d "$EPHYRA/crates/ephyra" ] || EPHYRA=/agent-run/ephyra
[ -d "$EPHYRA/crates/ephyra" ] || { echo "ephyra not found (set EPHYRA to a checkout with crates/ephyra)" >&2; exit 1; }
HOST="$(rustc -vV | sed -n 's/^host: //p')"

CRATE="$BUILD/crate"
mkdir -p "$CRATE/src"
# copy sources only when changed so cargo's fingerprints stay valid
for f in "$HERE"/crate/src/*.rs; do
  cmp -s "$f" "$CRATE/src/$(basename "$f")" || cp "$f" "$CRATE/src/"
done
sed -e "s#@SRC@#$SRC#g" -e "s#@EPHYRA@#$EPHYRA#g" "$HERE/crate/Cargo.toml.in" > "$CRATE/Cargo.toml.new"
cmp -s "$CRATE/Cargo.toml.new" "$CRATE/Cargo.toml" || mv "$CRATE/Cargo.toml.new" "$CRATE/Cargo.toml"
rm -f "$CRATE/Cargo.toml.new"
# resolve the same dependency versions as the target's own workspace
[ -f "$CRATE/Cargo.lock" ] || cp "$SRC/Cargo.lock" "$CRATE/Cargo.lock" 2>/dev/null || true
mkdir -p "$BUILD/bin"

if [ "$MODE" = coverage ]; then
  SYSROOT="$(rustc --print sysroot)"
  TOOLS="$SYSROOT/lib/rustlib/$HOST/bin"
  [ -x "$TOOLS/llvm-profdata" ] && [ -x "$TOOLS/llvm-cov" ] || { echo "llvm-tools missing in $TOOLS (rustup component add llvm-tools)" >&2; exit 1; }
  # an explicit --target keeps the flag off build scripts and proc macros
  export RUSTFLAGS="${RUSTFLAGS:-} --cfg fuzzing -C instrument-coverage"
  # instrumented build scripts would otherwise drop .profraw files into the source tree
  export LLVM_PROFILE_FILE="$BUILD/profraw-build/%m_%p.profraw"
  cargo build --manifest-path "$CRATE/Cargo.toml" --profile fuzz --target "$HOST" --target-dir "$BUILD/target-cov" 2>&1 | tail -n 30
  cp "$BUILD/target-cov/$HOST/fuzz/bbz" "$BUILD/bin/bbz.new"
  mv "$BUILD/bin/bbz.new" "$BUILD/bin/bbz"
  printf '{"objects": ["%s"], "profdata": "%s", "cov": "%s"}\n' \
    "$BUILD/bin/bbz" "$TOOLS/llvm-profdata" "$TOOLS/llvm-cov" > "$BUILD/coverage.json"
  echo "wrote $BUILD/coverage.json"
  echo "built $BUILD/bin/bbz (coverage)"
  exit 0
fi

export RUSTFLAGS="${RUSTFLAGS:-} --cfg fuzzing"
cargo build --manifest-path "$CRATE/Cargo.toml" --profile fuzz --target-dir "$BUILD/target" 2>&1 | tail -n 30
cp "$BUILD/target/fuzz/bbz" "$BUILD/bin/bbz.new"
mv "$BUILD/bin/bbz.new" "$BUILD/bin/bbz"
echo "built $BUILD/bin/bbz"

# AddressSanitizer build: run.sh uses it for odd seeds. Optional.
asan() {
  if ! rustup toolchain list 2>/dev/null | grep -q '^nightly'; then
    rustup toolchain install nightly --profile minimal > "$BUILD/asan-toolchain.log" 2>&1 || return 1
  fi
  RUSTFLAGS="--cfg fuzzing -Zsanitizer=address" cargo +nightly build --manifest-path "$CRATE/Cargo.toml" \
    --profile fuzz --target "$HOST" --target-dir "$BUILD/target-asan" > "$BUILD/asan-build.log" 2>&1 || return 1
  cp "$BUILD/target-asan/$HOST/fuzz/bbz" "$BUILD/bin/bbz-asan.new"
  mv "$BUILD/bin/bbz-asan.new" "$BUILD/bin/bbz-asan"
}
# Schedule-perturbation build: run.sh uses it for seeds = 2 (mod 4). A copy of the sources whose
# Interleaving::switch_point / wait (bbtree/src/tree/interleaving.rs, Shuttle's switch points) call the
# harness's hooks (crate/src/chaos.rs). Optional; BBZ_NO_CHAOS=1 skips it.
chaos() {
  local CS="$BUILD/chaos-src" CC="$BUILD/crate-chaos" IL
  mkdir -p "$CS" "$CC/src"
  rsync -a --delete --exclude target --exclude .git --exclude /bbtree/src/tree/interleaving.rs \
    --include '/Cargo.toml' --include '/Cargo.lock' --include '/bbtree/***' --include '/bbtree-mmap/***' \
    --include '/bbtree-store/***' --exclude '*' "$SRC/" "$CS/" || return 1
  IL="$CS/bbtree/src/tree/interleaving.rs"
  python3 - "$SRC/bbtree/src/tree/interleaving.rs" "$IL.new" <<'PY' || return 1
import sys
s = open(sys.argv[1]).read()
sp = "pub(in crate::tree) fn switch_point() {\n"
wt = "pub(in crate::tree) fn wait(spins: u32) {\n"
assert s.count(sp) == 1 and s.count(wt) == 1, "interleaving.rs changed shape"
s = s.replace(sp, sp + '        unsafe extern "Rust" { safe fn bbz_chaos_switch_point(); }\n        bbz_chaos_switch_point();\n')
s = s.replace(wt, wt + '        unsafe extern "Rust" { safe fn bbz_chaos_wait(spins: u32); }\n        if true { bbz_chaos_wait(spins); return; }\n')
open(sys.argv[2], "w").write(s)
PY
  if cmp -s "$IL.new" "$IL"; then rm -f "$IL.new"; else mv "$IL.new" "$IL"; fi
  for f in "$HERE"/crate/src/*.rs; do
    cmp -s "$f" "$CC/src/$(basename "$f")" || cp "$f" "$CC/src/"
  done
  sed -e "s#@SRC@#$CS#g" -e "s#@EPHYRA@#$EPHYRA#g" "$HERE/crate/Cargo.toml.in" > "$CC/Cargo.toml.new"
  if cmp -s "$CC/Cargo.toml.new" "$CC/Cargo.toml"; then rm -f "$CC/Cargo.toml.new"; else mv "$CC/Cargo.toml.new" "$CC/Cargo.toml"; fi
  [ -f "$CC/Cargo.lock" ] || cp "$CRATE/Cargo.lock" "$CC/Cargo.lock" 2>/dev/null || true
  RUSTFLAGS="--cfg fuzzing --cfg bbz_chaos" cargo build --manifest-path "$CC/Cargo.toml" --profile fuzz \
    --target-dir "$BUILD/target-chaos" > "$BUILD/chaos-build.log" 2>&1 || return 1
  cp "$BUILD/target-chaos/fuzz/bbz" "$BUILD/bin/bbz-chaos.new"
  mv "$BUILD/bin/bbz-chaos.new" "$BUILD/bin/bbz-chaos"
}
# built alongside the ASan build
chaos_build() {
  if [ "${BBZ_NO_CHAOS:-}" = 1 ]; then
    rm -f "$BUILD/bin/bbz-chaos"
  elif chaos; then
    echo "built $BUILD/bin/bbz-chaos"
  else
    rm -f "$BUILD/bin/bbz-chaos"
    echo "warning: no schedule-perturbation build (see $BUILD/chaos-build.log); run.sh uses bbz for its seeds" >&2
  fi
}
chaos_build &
CHAOS_PID=$!

if [ "${BBZ_NO_ASAN:-}" = 1 ]; then
  rm -f "$BUILD/bin/bbz-asan"
elif asan; then
  echo "built $BUILD/bin/bbz-asan"
else
  rm -f "$BUILD/bin/bbz-asan"
  echo "warning: no AddressSanitizer build (see $BUILD/asan-build.log or asan-toolchain.log); run.sh uses bbz for every seed" >&2
fi
wait "$CHAOS_PID"
