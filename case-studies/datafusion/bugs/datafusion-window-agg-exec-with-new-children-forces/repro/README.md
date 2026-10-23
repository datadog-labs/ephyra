# Repro: SanityCheckPlan error for two window functions with target_partitions = 1

    ./repro.sh /path/to/datafusion-cli     # or: datafusion-cli -f repro.sql

Exit 1: the bug reproduces (the output says "does not satisfy distribution requirements: KeyPartitioned[[NULL]]").
Exit 0: the query returns two rows `2,1` (the expected result). Exit 2: cannot decide.

Results:
- main 3bc3d0bb822f (2026-10-21), debug build: exit 1
- release 55.2.0: exit 1
- main plus the fix (WindowAggExec::with_new_children keeps can_repartition): exit 0
- sqlite3 3.45.1 runs the same SELECT and returns 2|1 twice
