#!/usr/bin/env python3
"""Regenerate ../sections.tsv, the function-to-file map of the vendored
SQLite amalgamation, from its "Begin file" and "Continuing where we left
off" markers (the map format_counters reads).

    python3 deploy/sections.py
"""
import os
import re

here = os.path.dirname(os.path.abspath(__file__))
cov = os.path.dirname(here)
amalgamation = os.path.join(cov, "..", "vendor", "libsqlite3-sys-trunk", "sqlite3", "sqlite3.c")
header = os.path.join(cov, "..", "vendor", "libsqlite3-sys-trunk", "sqlite3", "sqlite3.h")
source_id = next(l for l in open(header) if "define SQLITE_SOURCE_ID" in l).split()[4][:12]

begin = re.compile(r"/\*+ Begin file (\S+) \*+/")
cont = re.compile(r"/\*+ Continuing where we left off in (\S+) \*+/")
defn = re.compile(r"^[A-Za-z_][\w \*]*?\b([A-Za-z_]\w*)\s*\(")
section, out = None, {}
for line in open(amalgamation, encoding="latin-1").read().split("\n"):
    m = begin.search(line) or cont.search(line)
    if m:
        section = m.group(1)
        continue
    if not line or line[0] in " \t#/}{*" or line.startswith("typedef") or line.startswith("struct"):
        continue
    m = defn.match(line)
    if m and not line.rstrip().endswith(";"):
        name = m.group(1)
        if name not in ("if", "while", "for", "switch", "return", "sizeof"):
            out.setdefault(name, section)
with open(os.path.join(cov, "sections.tsv"), "w") as f:
    f.write(f'# Function name -> source file inside the SQLite amalgamation of ../vendor/libsqlite3-sys-trunk (trunk {source_id}), from its "Begin file" and "Continuing where we left off" markers.\n')
    for k, v in sorted(out.items()):
        f.write(f"{k}\t{v}\n")
print(f"{len(out)} functions mapped (trunk {source_id})")
