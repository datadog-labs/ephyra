#!/usr/bin/env bash
# fmt: a paragraph's first word ending a sentence at end of line is joined with one space, not two.
#   repro.sh <dir-with-fmt-binary-or-coreutils-multicall> [gnu-fmt]
# Exit 1 = bug reproduces, 0 = does not, 2 = cannot run.
# The expected output is hard-coded; if a GNU fmt path is given its output is shown as well.
set -u
D=${1:?usage: repro.sh <bin-dir> [gnu-fmt]}
if [ -x "$D/fmt" ]; then FMT=("$D/fmt"); elif [ -x "$D/coreutils" ]; then FMT=("$D/coreutils" fmt)
else echo "no fmt or coreutils in $D" >&2; exit 2; fi
GNU=${2:-}
export LC_ALL=C
fail=0
check() { # $1 = label, $2 = input (printf format), $3 = expected stdout, rest = fmt args
  local label=$1 in=$2 want=$3; shift 3
  local got; got=$(printf "$in" | "${FMT[@]}" "$@") || { echo "fmt failed" >&2; exit 2; }
  local g=""; [ -n "$GNU" ] && g="  gnu=[$(printf "$in" | "$GNU" "$@")]"
  if [ "$got" = "$want" ]; then s=ok; else s=WRONG; fail=1; fi
  printf '%-6s %-28s got=[%s] want=[%s]%s\n' "$s" "$label" "$got" "$want" "$g"
}
check "fmt        'a.\\nz'"        'a.\nz\n'        'a.  z'
check "fmt -u     'a.\\nz'"        'a.\nz\n'        'a.  z'        -u
check "fmt        'Hello.\\nWorld.'" 'Hello.\nWorld.\n' 'Hello.  World.'
check "fmt        'Why?\\nnot'"     'Why?\nnot\n'    'Why?  not'
check "(control) 'x a.\\nz'"       'x a.\nz\n'      'x a.  z'
exit $fail
