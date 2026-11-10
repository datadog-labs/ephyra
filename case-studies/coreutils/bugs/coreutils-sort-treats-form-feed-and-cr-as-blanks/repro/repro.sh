#!/usr/bin/env bash
# sort: form feed / carriage return treated as blanks (C locale).
# Usage: repro.sh <path-to-sort-or-coreutils-multicall> [gnu-sort]
#   exit 1 = bug reproduces (some case differs from GNU 9.12 output), 0 = all match, 2 = usage error.
# Expected outputs below were recorded from GNU coreutils 9.12 with LC_ALL=C.
set -u
BIN=${1:?usage: repro.sh <uutils sort or coreutils binary>}
[ -x "$BIN" ] || exit 2
case "$(basename "$BIN")" in coreutils*) SORT=("$BIN" sort) ;; *) SORT=("$BIN") ;; esac
export LC_ALL=C
fail=0
check() { # $1 = description, $2 = input (printf format), $3 = expected (printf format), rest = sort args
  local desc=$1 in=$2 want=$3; shift 3
  local got exp
  got=$(printf "$in" | "${SORT[@]}" "$@" | od -An -c | tr -s ' ')
  exp=$(printf "$want" | od -An -c | tr -s ' ')
  if [ "$got" = "$exp" ]; then echo "ok    $desc"; else echo "DIFF  $desc"; echo "   expected:$exp"; echo "   actual:  $got"; fail=1; fi
}
# \f is not a blank, so "\f2" is not a number (value 0); the last-resort byte compare puts \f (0x0C) before b.
check "sort -n, form feed before digit"      'b\n\f2\n'   '\f2\nb\n' -n
check "sort -n, carriage return before digit" 'b\n\r2\n'   '\r2\nb\n' -n
# -b must skip only space/tab: the key of "\fb" keeps its \f (0x0C), which sorts before "a".
check "sort -b, form feed is not a leading blank" ' a\n\fb\n' '\fb\n a\n' -b
# default field splitting: \r is not a separator, so "c\rz" is one field and its key 2 is empty.
check "sort -k2b, carriage return is not a field separator" 'a b\nc\rz\n' 'c\rz\na b\n' -k2b
# -d keeps "blanks and alphanumerics": \f is neither, so "a\fc" compares as "ac", after "ab".
check "sort -d, form feed is ignored like other non-blank punctuation" 'a\fc\nab\n' 'ab\na\fc\n' -d
# -M skips leading blanks only: "\fFEB" is not a month, and unknown sorts before JAN.
check "sort -M, form feed before month name" '\fFEB\nJAN\n' '\fFEB\nJAN\n' -M
check "sort -h, form feed before number" 'b\n\f2K\n' '\f2K\nb\n' -h
exit $fail
