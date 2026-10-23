# Reproducer

    ./repro.sh /path/to/datafusion-checkout

This builds `src/main.rs` against the checkout's `datafusion/core` (path dependency) in `$TMPDIR/sortexec-fetch-zero-repro`
(set `REPRO_WORK` to change it), then runs it. The first build takes about 10 minutes.

Exit 1 means the bug reproduces (panic `assertion failed: k > 0`). Exit 0 means the query returned 0 rows. Exit 2 means
the build failed or the output was unexpected.
