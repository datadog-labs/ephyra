# Repro: SanityCheckPlan with sort-merge join and `a + b = a`

    ./repro.sh /path/to/datafusion-cli

Runs `repro.sql` from an empty temp directory and prints the output.
Exit 1 = bug reproduces (SanityCheckPlan error), 0 = the expected row `1 | 0 | 1 | 0` comes back, 2 = cannot decide.

Checked (2026-10-22):
- main 6489d1510ce1f38c14491e520d152986a8cb5c5d, `cargo build --profile ci -p datafusion-cli`: exit 1
- datafusion-cli 55.2.0: exit 1
- the same main with the code part of ../fix.diff applied: exit 0

The number in `Hash([a@0, b@1], 16)` in the error is the machine's CPU count (target_partitions).
