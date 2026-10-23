#!/usr/bin/env bash
# Usage: repro.sh <path-to-brush> [<path-to-bash-5.3>]
# Exit 1 if brush drops the leading zeros of an unexpandable ~N / ~+N / ~-N tilde-prefix, 0 if not.
BRUSH=${1:?usage: repro.sh <path-to-brush> [<path-to-bash>]}
REF=${2:-}
SCRIPT='echo ~01 ~+01 ~-007 ~{01..3} ~01/x
D=~+02; echo "D: $D"'
EXPECTED='~01 ~+01 ~-007 ~01 ~02 ~03 ~01/x
D: ~+02'
run() { env -i LC_ALL=C PATH=/usr/bin:/bin "$@" -c "$SCRIPT"; }
actual=$(cd "$(mktemp -d)" && run "$BRUSH" --norc --noprofile --no-config)
echo "--- brush:"; printf '%s\n' "$actual"
if [ -n "$REF" ]; then echo "--- bash:"; (cd "$(mktemp -d)" && run "$REF" --norc --noprofile); fi
echo "--- expected (bash 5.3):"; printf '%s\n' "$EXPECTED"
[ "$actual" = "$EXPECTED" ] && { echo "OK: not reproduced"; exit 0; }
echo "BUG: reproduced"; exit 1
