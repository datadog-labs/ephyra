#!/usr/bin/env bash
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

# Run every known-bugs.json check on one build, in a throwaway cluster:
# OK when the output equals the bug's `expected` (what a build with the fix
# prints), else DIFF and the output. For a build-fixed.sh build every bug
# should be OK.
#
#     run-checks.sh BINDIR [PORT]
set -u
B=$1 PORT=${2:-5699}
REGISTRY="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/known-bugs.json"
T=$(mktemp -d)
"$B/initdb" -D "$T/data" -U postgres --locale=C -E UTF8 > /dev/null 2>&1
cat >> "$T/data/postgresql.conf" <<EOF
port = $PORT
unix_socket_directories = '$T'
listen_addresses = ''
fsync = off
dynamic_shared_memory_type = mmap
EOF
"$B/pg_ctl" -D "$T/data" -l "$T/log" -w start > /dev/null || { tail -5 "$T/log"; exit 1; }
export PATH="$B:$PATH" PGHOST="$T" PGPORT="$PORT" PGUSER=postgres
python3 - "$REGISTRY" <<'EOF'
import json, subprocess, sys
reg = json.load(open(sys.argv[1]))
for i, b in enumerate(reg["bugs"]):
    db = "c%d" % i
    subprocess.run(["createdb", db])
    try:
        out = subprocess.run(["psql", "-X", "-q", "-At", "-d", db, "-v", "ON_ERROR_STOP=0", "-v", "VERBOSITY=sqlstate", "-f", "-"],
                             input=b["check"], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=120)
        got = out.stdout
    except subprocess.TimeoutExpired:
        got = "TIMEOUT"
    print(b["id"], "OK" if got == b["expected"] else "DIFF " + repr(got[:300]))
EOF
"$B/pg_ctl" -D "$T/data" stop -m immediate > /dev/null 2>&1
rm -rf "$T"
