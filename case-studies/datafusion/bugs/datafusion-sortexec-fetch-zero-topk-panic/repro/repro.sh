#!/usr/bin/env bash
# usage: repro.sh <path-to-datafusion-checkout>
# Builds a tiny binary against that checkout and runs it.
# Exit 1 = bug reproduces (panic "assertion failed: k > 0"), 0 = query returns 0 rows, 2 = could not decide.
set -u
DF=$(cd "${1:?usage: repro.sh <datafusion-checkout>}" && pwd) || exit 2
HERE=$(cd "$(dirname "$0")" && pwd)
WORK=${REPRO_WORK:-${TMPDIR:-/tmp}/sortexec-fetch-zero-repro}
mkdir -p "$WORK/src"
sed "s#@DATAFUSION@#$DF#" "$HERE/Cargo.toml.in" > "$WORK/Cargo.toml"
cp "$HERE/src/main.rs" "$WORK/src/main.rs"
cargo build --quiet --manifest-path "$WORK/Cargo.toml" || exit 2
OUT=$("$WORK/target/debug/sortexec-fetch-zero-repro" 2>&1); RC=$?
echo "$OUT"
if grep -q "assertion failed: k > 0" <<<"$OUT"; then exit 1; fi
if [ $RC -eq 0 ] && grep -q "rows returned: 0" <<<"$OUT"; then exit 0; fi
exit 2
