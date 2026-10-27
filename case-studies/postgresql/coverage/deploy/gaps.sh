#!/usr/bin/env bash
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

# The gap list of the generator: what of PostgreSQL's backend its inputs
# never reach, for steering rounds (the PostgreSQL runner is not
# instrumented, so this measures the generator of this checkout on the
# scorer's instrumented build instead of a run).
#
#     gaps.sh OUT_DIR [SEED] [INPUTS]        (defaults: seed 0, 2000 inputs)
#
# Stops this checkout's qscore cluster (.pg) and clears its profiles, runs
# `qscore --seed S --inputs N --dirs OUT/dirs.tsv` (a fresh profile), stops
# the cluster again (the postmaster writes the merged profile at exit),
# merges it with llvm-profdata, exports a per-file summary with llvm-cov
# against the instrumented postgres binary, and writes OUT/gaps.md: totals,
# the src/backend files with the most missed lines, files never entered, and
# qscore's per-directory scores. Standalone (no runner.env): the managed
# controller runs it in a gate worktree, in its sandbox. The instrumented
# build is $PGCOV_BIN (default ~/Projects/postgres-cov/inst/bin, the
# scorer's); LLVM_PROFDATA and LLVM_COV name the tools of the LLVM that
# built it (default llvm-profdata / llvm-cov on PATH, else xcrun's).
set -euo pipefail
OUT=${1:?usage: gaps.sh OUT_DIR [SEED] [INPUTS]}
SEED=${2:-0}
N=${3:-2000}
COV="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="${PGCOV_BIN:-$HOME/Projects/postgres-cov/inst/bin}"
tool() { if command -v "$1" > /dev/null; then echo "$1"; else echo "xcrun $1"; fi; }
PROFDATA=${LLVM_PROFDATA:-$(tool llvm-profdata)}
COVTOOL=${LLVM_COV:-$(tool llvm-cov)}
mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"
cd "$COV"
if [ -f .pg/data/postmaster.pid ]; then "$BIN/pg_ctl" -D .pg/data stop -m fast > /dev/null 2>&1 || true; fi
mkdir -p .pg/prof
find .pg/prof -name '*.profraw' -delete   # thousands of files: never a shell glob
cargo run --release -q --bin qscore -- --seed "$SEED" --inputs "$N" --dirs "$OUT/dirs.tsv" > "$OUT/qscore.txt"
"$BIN/pg_ctl" -D .pg/data stop -m fast > /dev/null
find "$PWD/.pg/prof" -name '*.profraw' > "$OUT/profraw.list"
$PROFDATA merge -sparse -f "$OUT/profraw.list" -o "$OUT/gaps.profdata"
$COVTOOL export -summary-only -instr-profile "$OUT/gaps.profdata" "$BIN/postgres" > "$OUT/summary.json"
python3 - "$OUT" "$SEED" "$N" <<'PY'
import json, os, sys
out, seed, n = sys.argv[1], sys.argv[2], sys.argv[3]
files = json.load(open(os.path.join(out, "summary.json")))["data"][0]["files"]
# Out of scope: no SQL session reaches them, or not a target (gaps/2026-10-16.md).
SKIP = ("replication/", "backup/", "bootstrap/", "postmaster/", "access/rmgrdesc/", "libpq/", "xlogrecovery.c",
        "tsearch/dict_", "utils/adt/xml.c", "utils/adt/pgstatfuncs.c", "commands/subscriptioncmds.c",
        "commands/publicationcmds.c", "commands/tablespace.c", "commands/async.c", "_xlog.c", "redo",
        # generated code: the grammar's tables and the node functions say little about features
        "parser/gram.c", "parser/scan.c", "nodes/outfuncs.funcs.c", "nodes/readfuncs.funcs.c",
        "nodes/copyfuncs.funcs.c", "nodes/equalfuncs.funcs.c", "nodes/queryjumblefuncs.funcs.c")
rows = []
tot = {"lines": [0, 0], "functions": [0, 0]}
for f in files:
    name = f["filename"]
    i = name.find("/src/backend/")
    if i < 0:
        continue
    rel = name[i + len("/src/backend/"):]
    s = f["summary"]
    for k in tot:
        tot[k][0] += s[k]["covered"]
        tot[k][1] += s[k]["count"]
    if any(x in rel for x in SKIP):
        continue
    rows.append((s["lines"]["count"] - s["lines"]["covered"], rel, s))
rows.sort(key=lambda r: -r[0])
q = open(os.path.join(out, "qscore.txt")).read().splitlines()
lines = [f"# Coverage gaps of the generator (qscore seed {seed}, {n} inputs, fresh profile)", "",
         "qscore: " + (q[0] if q else "?"), (q[1] if len(q) > 1 else ""), "",
         f"src/backend: lines {tot['lines'][0]}/{tot['lines'][1]}, functions {tot['functions'][0]}/{tot['functions'][1]} "
         "reached by at least one input. \"Missed\" means no input of this sample executed it: rare paths count as missed too.",
         "Out of scope and left out below: " + ", ".join(SKIP) + ".", "",
         "## Files by missed lines (top 80)", "", "| file | missed lines | lines | functions |", "|---|---|---|---|"]
for missed, rel, s in rows[:80]:
    lines.append(f"| {rel} | {missed} | {s['lines']['covered']}/{s['lines']['count']} | "
                 f"{s['functions']['covered']}/{s['functions']['count']} |")
never = [rel for missed, rel, s in rows if s["functions"]["covered"] == 0 and s["functions"]["count"] > 0]
lines += ["", f"## Files never entered ({len(never)})", ""] + [f"- {x}" for x in never[:150]]
d = os.path.join(out, "dirs.tsv")
if os.path.exists(d):
    lines += ["", "## qscore by directory", "", "```"] + open(d).read().splitlines()[:60] + ["```"]
open(os.path.join(out, "gaps.md"), "w").write("\n".join(lines) + "\n")
print(os.path.join(out, "gaps.md"))
PY
