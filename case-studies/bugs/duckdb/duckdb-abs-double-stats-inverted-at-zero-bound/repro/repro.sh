#!/bin/bash
# usage: repro.sh <path-to-duckdb-cli>
# exit 1 = wrong result reproduces (row c=0.0 gives false), 0 = correct (true), 2 = cannot decide
D=$(cd "$(dirname "$0")" && pwd)
CLI=${1:?usage: repro.sh <path-to-duckdb-cli>}
OUT=$("$CLI" -csv :memory: < "$D/repro.sql" 2>&1)
echo "$OUT"
case "$(echo "$OUT" | grep '^0.0,')" in
  0.0,false) exit 1 ;;
  0.0,true) exit 0 ;;
  *) exit 2 ;;
esac
