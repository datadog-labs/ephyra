#!/usr/bin/env bash
# usage: run.sh <dir> [api_version]
#   <dir> is either an upstream CMake build dir (bin/fdbserver, bin/fdbcli, lib/libfdb_c.so, bindings/python)
#   or a directory with release binaries fdbserver, fdbcli, libfdb_c.so and a python binding under py/.
# Starts a one-process memory cluster in a temp dir, runs poc.py, stops the cluster.
# Exit 1 = bug reproduces, 0 = correct, other = could not run.
set -u
D=$(realpath -e "$1") || exit 2
API=${2:-730}
HERE=$(cd "$(dirname "$0")" && pwd)
if [ -x "$D/bin/fdbserver" ]; then
  SERVER=$D/bin/fdbserver CLI=$D/bin/fdbcli LIB=$D/lib PY=$D/bindings/python
else
  SERVER=$D/fdbserver CLI=$D/fdbcli LIB=$D PY=$(ls -d "$D"/py/foundationdb-*/ | head -1)
fi
T=$(mktemp -d)
PORT=$((20000 + RANDOM % 20000))
echo "test:test@127.0.0.1:$PORT" > "$T/fdb.cluster"
mkdir -p "$T/data" "$T/logs"
"$SERVER" -p 127.0.0.1:$PORT -C "$T/fdb.cluster" -d "$T/data" -L "$T/logs" > "$T/server.out" 2>&1 &
SPID=$!
trap 'kill $SPID 2>/dev/null; wait $SPID 2>/dev/null; rm -rf "$T"' EXIT
"$CLI" -C "$T/fdb.cluster" --exec "configure new single memory" --timeout 60 > /dev/null || exit 2
PYTHONPATH=$PY LD_LIBRARY_PATH=$LIB python3 "$HERE/poc.py" "$T/fdb.cluster" "$API"
