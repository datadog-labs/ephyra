#!/usr/bin/env bash
# variants.sh <dogwood-bin>: for each pair, print SAME if explain prose is identical
here="$(cd "$(dirname "$0")" && pwd)"; D=$1; S=$here/schema.cedarschema; C='context.input.n == 0'
pair() { # name grouped regrouped
  printf 'permit (principal, action, resource)\nwhen { %s };\n' "$2" > ${TMPDIR:-/tmp}/v_g.dw
  printf 'permit (principal, action, resource)\nwhen { %s };\n' "$3" > ${TMPDIR:-/tmp}/v_r.dw
  g=$($D explain ${TMPDIR:-/tmp}/v_g.dw --policy-schema $S 2>&1); r=$($D explain ${TMPDIR:-/tmp}/v_r.dw --policy-schema $S 2>&1)
  vg=$($D replay ${TMPDIR:-/tmp}/v_g.dw --policy-schema $S --trace $here/trace.log 2>&1 | head -1 | grep -o 'ALLOW\|DENY')
  vr=$($D replay ${TMPDIR:-/tmp}/v_r.dw --policy-schema $S --trace $here/trace.log 2>&1 | head -1 | grep -o 'ALLOW\|DENY')
  [ "$g" = "$r" ] && s=SAME || s=differ
  echo "$s  @0 $vg/$vr  $1"
}
pair 'not-and-left'   "!((if $C then true else false) && false)" "!(if $C then true else (false && false))"
pair 'not-or-left'    "!((if $C then false else false) || true)" "!(if $C then false else (false || true))"
pair 'top-and-left'   "(if $C then true else false) && false" "if $C then true else (false && false)"
pair 'top-or-left'    "(if $C then false else false) || true" "if $C then false else (false || true)"
pair 'nested-or-in-and' "((if $C then false else false) || true) && true" "(if $C then false else (false || true)) && true"
pair 'eq-true-and'    "((if $C then true else false) && false) == true" "(if $C then true else (false && false)) == true"
pair 'not-and-right-in-or' "!((true && (if $C then false else true)) || true)" "!(true && (if $C then false else (true || true)))"
