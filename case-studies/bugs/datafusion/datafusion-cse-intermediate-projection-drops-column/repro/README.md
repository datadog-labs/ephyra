Run against a datafusion-cli built from upstream `main`:

    ./repro.sh /path/to/datafusion-cli      # exit 1 = bug, 0 = correct row, 2 = undecided

`repro.sql` is the deterministic schema-error case. `panic.sql` hits the same bug but panics in about half
the runs, depending on HashMap order: `datafusion-cli -q -f panic.sql`.
Checked: main 4f2dbf70be7e -> exit 1; main + fix.diff -> exit 0 (prints 1 | 2 | 4).
