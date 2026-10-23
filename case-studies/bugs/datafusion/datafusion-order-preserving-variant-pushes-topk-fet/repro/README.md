# Repro: count(*) OVER () sees only the LIMIT rows (target_partitions > 1)

    ./repro.sh /path/to/datafusion-cli     # or: datafusion-cli -f repro.sql

Exit 1: the bug reproduces (output row `1,1`). Exit 0: correct (`3,1`). Exit 2: cannot decide.

Results:
- main 86e10a5499b1 (2026-10-21), debug build: exit 1
- main 3bc3d0bb822f: exit 1
- release 55.2.0: exit 1
- same SQL with target_partitions = 1 on main: 3,1
- DuckDB 1.5.6 and sqlite3 (SELECT only): 3,1
