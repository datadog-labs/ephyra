# Reproducer

    ./repro.sh /path/to/datafusion-cli

The binary must be built with debug assertions, for example `cargo build -p datafusion-cli` (dev profile) or
`cargo build --profile ci -p datafusion-cli`. Release builds skip the `debug_assert!` and return the right rows.

`repro.sql` is the whole case. Exit 1 means the bug reproduces (panic "all window expressions shall have the same
sort keys"). Exit 0 means the query returned the expected rows `2,10` and `4,30`. Exit 2 means anything else.
Run it from a directory that does not hold stray data files.
