#!/usr/bin/env bash
# brush must brace-expand assignment-form arguments of declaration builtins like bash 5.3:
#   `local v={c,d}$x` (x='a 1-') is `local v=c$x v=d$x`, and each word is then split,
#   so bash rejects the word `1-' twice on stderr ("not a valid identifier") and ends with v="da".
# brush instead assigns the literal v="{c,d}a 1-" and prints nothing on stderr.
# Exit 1 = reproduces, 0 = fixed, 2 = cannot decide.
cd "$(dirname "$0")" || exit 2
B=${1:?usage: repro.sh <build-dir>}
[ -x "$B/brush" ] || exit 2
t=$(mktemp -d) || exit 2
trap 'rm -rf "$t"' EXIT
(cd "$t" && timeout 10 "$B/brush" --norc --noprofile "$OLDPWD/script.sh" >out 2>err)
[ -f "$t/out" ] || exit 2
out=$(cat "$t/out"); nerr=$(grep -c "1-" "$t/err")
if [ "$out" = 'declare -- v="da"' ] && [ "$nerr" = 2 ]; then exit 0; fi
echo "stdout: $out"; echo "stderr:"; cat "$t/err"
exit 1
