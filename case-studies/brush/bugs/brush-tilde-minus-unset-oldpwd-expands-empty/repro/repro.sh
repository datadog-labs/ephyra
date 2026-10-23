#!/usr/bin/env bash
# Usage: repro.sh <path-to-brush> [<path-to-bash-5.3>]
# Exit 1 if brush expands `~-` to "" while OLDPWD is declared but unset (the startup state when OLDPWD
# is not inherited from the environment), 0 if not.
BRUSH=${1:?usage: repro.sh <path-to-brush> [<path-to-bash>]}
REF=${2:-}
SCRIPT='printf "<%s>\n" ~- ~-/x
unset OLDPWD; declare -x OLDPWD; printf "<%s>\n" ~-'
EXPECTED='<~->
<~-/x>
<~->'
# env -i: OLDPWD is not inherited, so the shell starts with OLDPWD declared but unset.
run() { env -i LC_ALL=C PATH=/usr/bin:/bin "$@" -c "$SCRIPT"; }
actual=$(cd "$(mktemp -d)" && run "$BRUSH" --norc --noprofile --no-config)
echo "--- brush:"; printf '%s\n' "$actual"
if [ -n "$REF" ]; then echo "--- bash:"; (cd "$(mktemp -d)" && run "$REF" --norc --noprofile); fi
echo "--- expected (bash 5.3):"; printf '%s\n' "$EXPECTED"
[ "$actual" = "$EXPECTED" ] && { echo "OK: not reproduced"; exit 0; }
echo "BUG: reproduced"; exit 1
