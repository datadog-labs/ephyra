#!/usr/bin/env bash
# run.sh <bbtree-checkout>: builds main.rs against that checkout's bbtree and runs it.
# Exit 1 if pages are still out after the map is dropped (the bug), 0 if not.
set -u
SRC="$(cd "$1" && pwd)" || exit 2
HERE="$(cd "$(dirname "$0")" && pwd)"
P="$(mktemp -d)"
mkdir -p "$P/src"; cp "$HERE/main.rs" "$P/src/main.rs"
cat > "$P/Cargo.toml" <<TOML
[package]
name = "free-list-leak"
version = "0.1.0"
edition = "2024"
[dependencies]
bbtree = { path = "$SRC/bbtree" }
[workspace]
TOML
out="$(cargo run -q --release --manifest-path "$P/Cargo.toml" --target-dir "${CARGO_TARGET_DIR:-$P/target}" 2>/dev/null)" || exit 2
echo "$out"
case "$out" in *"pages out after drop: 0"*) exit 0 ;; *) exit 1 ;; esac
