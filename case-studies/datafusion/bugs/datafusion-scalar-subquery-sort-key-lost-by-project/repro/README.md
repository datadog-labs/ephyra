# Reproducer

    ./repro.sh /path/to/datafusion-cli          # an existing binary
    ./repro.sh /path/to/datafusion-checkout     # builds datafusion-cli (release) first

It runs `repro.sql` with `datafusion-cli -q -f`.

- Exit 1: the bug reproduces. The `SanityCheckPlan ... does not satisfy order requirements` error is printed to stderr.
- Exit 0: the query returned the expected rows `(true,3) (true,3) (false,3)`.
- Exit 2: the build failed or the output was unexpected. The output is printed.

To see the control, delete the `SET datafusion.execution.batch_size = 1;` line: the query then succeeds.
