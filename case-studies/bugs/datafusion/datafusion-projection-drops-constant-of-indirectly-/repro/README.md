# Repro: SanityCheckPlan error, sort-merge join + filter-constant ORDER BY expression

    ./repro.sh /path/to/datafusion-cli      # or a datafusion checkout (builds datafusion-cli, debug)

Exit 1 = bug reproduces (SanityCheckPlan "does not satisfy order requirements"),
0 = the correct row (2, 1) is returned, 2 = cannot decide.

Or by hand: `datafusion-cli -f repro.sql`. Remove the SET line to see the expected row with the default hash join.
