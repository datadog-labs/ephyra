#!/usr/bin/env bash
# fold: ASCII control characters (other than TAB/BS/CR) take no column.
#   repro.sh <dir-containing-fold | path-to-fold | path-to-coreutils-multicall>
# Exit 1 if any case differs from GNU coreutils 9.12 output (bug reproduces), 0 if all match,
# 2 on usage error. Expected bytes were recorded from GNU fold 9.12 with LC_ALL=C.
set -u
T=${1:?usage: repro.sh <dir-with-fold | fold binary | coreutils binary>}
if [ -d "$T" ]; then FOLD=("$T/fold"); elif [ "$(basename "$T")" = coreutils ]; then FOLD=("$T" fold); else FOLD=("$T"); fi
[ -x "${FOLD[0]}" ] || { echo "not executable: ${FOLD[0]}" >&2; exit 2; }

fail=0
check() { # $1 = label, $2 = printf input, $3 = printf expected output, rest = fold args
  local label=$1 in=$2 want=$3; shift 3
  local got exp
  got=$(printf "$in" | LC_ALL=C "${FOLD[@]}" "$@" | od -An -c | tr -s ' ')
  exp=$(printf "$want" | od -An -c | tr -s ' ')
  if [ "$got" = "$exp" ]; then echo "ok   $label"
  else echo "FAIL $label"; echo "     expected:$exp"; echo "     actual:  $got"; fail=1; fi
}

# Default (column) mode: a control character occupies one column, like any other character.
check "printf '\\f0' | fold -w1"                 '\f0'                '\f\n0'               -w1
check "printf '\\001\\007\\v\\033\\177' | fold -w2" '\001\007\v\033\177' '\001\007\n\v\033\n\177' -w2
check "printf 'abc\\001\\001\\001def' | fold -w3"   'abc\001\001\001def' 'abc\n\001\001\001\ndef' -w3
# -c mode: no extra newline when a control character exactly fills the line.
check "printf 'a\\001' | fold -c -w2"            'a\001'              'a\001'               -c -w2

exit $fail
