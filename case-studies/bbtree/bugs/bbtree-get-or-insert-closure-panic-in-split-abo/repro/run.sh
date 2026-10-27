#!/usr/bin/env bash
# run.sh <bbtree-checkout>: builds main.rs (debug) and leak.rs (release)
# against that checkout's bbtree crate and runs both.
# Exit 1 if the bug shows (debug build aborts, or release leaves pages
# allocated after drop), 0 if neither does.
set -u
SRC="$(cd "$1" && pwd)" || exit 2
HERE="$(cd "$(dirname "$0")" && pwd)"
P="$(mktemp -d)"
mkdir -p "$P/src/bin"
cp "$HERE/main.rs" "$P/src/main.rs"
cp "$HERE/leak.rs" "$P/src/bin/leak.rs"
cat > "$P/Cargo.toml" <<TOML
[package]
name = "get-or-insert-panic"
version = "0.1.0"
edition = "2024"
[dependencies]
bbtree = { path = "$SRC/bbtree" }
[workspace]
TOML
T="${CARGO_TARGET_DIR:-$P/target}"
cargo build -q --manifest-path "$P/Cargo.toml" --target-dir "$T" --bins || exit 2
cargo build -q --release --manifest-path "$P/Cargo.toml" --target-dir "$T" --bins || exit 2
bug=0
echo "== debug build"
env -u RUST_BACKTRACE "$T/debug/get-or-insert-panic"; rc=$?
echo "exit: $rc"
[ "$rc" -eq 0 ] || bug=1
echo "== release build, counting allocator"
out="$(env -u RUST_BACKTRACE "$T/release/leak")"; rc=$?
echo "$out"
[ "$rc" -eq 0 ] || exit 2
case "$out" in *"after drop: 0"*) ;; *) bug=1 ;; esac
exit $bug
