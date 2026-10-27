#!/usr/bin/env bash
# run.sh <bbtree-checkout>: builds main.rs against that checkout's bbtree-store and runs it.
# Exit 1 if the open fails with Corrupted (bug present), 0 if it succeeds.
set -eu
SRC="$(cd "$1" && pwd)"
HERE="$(cd "$(dirname "$0")" && pwd)"
P="$(mktemp -d)"
mkdir "$P/src"; cp "$HERE/main.rs" "$P/src/main.rs"
cat > "$P/Cargo.toml" <<TOML
[package]
name = "wal-open-repro"
version = "0.1.0"
edition = "2024"
[dependencies]
bbtree-store = { path = "$SRC/bbtree-store" }
[workspace]
TOML
out="$(cargo run -q --manifest-path "$P/Cargo.toml" --target-dir "${CARGO_TARGET_DIR:-$P/target}")"
echo "$out"
case "$out" in *Corrupted*) exit 1 ;; *) exit 0 ;; esac
