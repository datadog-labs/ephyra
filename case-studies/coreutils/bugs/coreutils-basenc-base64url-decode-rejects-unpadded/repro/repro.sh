#!/usr/bin/env bash
# basenc --base64url -d rejects input without '=' padding; basenc --base64 -d accepts it.
# Usage: repro.sh <dir-containing-basenc>   (e.g. a uutils build's bin dir, or GNU's bin dir)
# Exit 1 = bug reproduces (base64url rejects 'Pw' while base64 accepts it), 0 = does not.
B=${1:?usage: repro.sh <dir-containing-basenc>}
export LC_ALL=C
for enc in base64 base64url; do
  printf 'printf Pw | basenc --%s -d -> ' "$enc"
  out=$(printf Pw | "$B/basenc" --$enc -d 2>&1); rc=$?
  printf '%q exit %d\n' "$out" "$rc"
  eval "rc_$enc=$rc"
done
[ "$rc_base64" = 0 ] && [ "$rc_base64url" != 0 ] && exit 1
exit 0
