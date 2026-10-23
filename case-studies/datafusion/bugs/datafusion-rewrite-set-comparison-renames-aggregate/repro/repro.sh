#!/usr/bin/env bash
# usage: repro.sh <path-to-datafusion-cli>
# exit 1 = bug reproduces, 0 = does not, 2 = cannot decide
cd "$(dirname "$0")" || exit 2
CLI=${1:-datafusion-cli}
command -v "$CLI" >/dev/null 2>&1 || [ -x "$CLI" ] || exit 2
SETUP='CREATE TABLE t(x INT) AS VALUES (1), (6); CREATE TABLE s(y INT) AS VALUES (5);'
out=$("$CLI" -q --format csv -c "$SETUP SELECT x > ALL (SELECT y FROM s) FROM t;" 2>&1)
echo "$out"
# DuckDB returns false, true for this query.
if echo "$out" | grep -q "Optimizer rule"; then exit 1; fi
if [ "$(echo "$out" | tail -n 2 | tr '\n' ' ')" = "false true " ]; then exit 0; fi
exit 2
