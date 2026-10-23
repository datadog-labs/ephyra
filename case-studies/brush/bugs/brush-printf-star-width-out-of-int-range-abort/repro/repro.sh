#!/usr/bin/env bash
# Usage: repro.sh [shell]   (default: brush on PATH)
# printf with a `*` field width that does not fit in a C int.
# bash 5.3: diagnostic on stderr, then "[x]" and "rc=1".
# brush:    "printf: i/o error: formatting width too large", then only "[" and "rc=1".
SH=${1:-brush}
out=$(env -i LC_ALL=C PATH=/usr/bin:/bin "$SH" -c 'printf "[%*s]\n" 99999999999 x; echo "rc=$?"' 2>/dev/null)
printf '%s\n' "$out"
if [ "$out" = $'[x]\nrc=1' ]; then echo "OK: matches bash 5.3"; exit 0; else echo "BUG: output differs from bash 5.3"; exit 1; fi
