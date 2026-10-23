# Repro

`repro.sh <uutils coreutils binary | expand symlink> [<gnu expand>]`. It exits 1 when the bug
reproduces, 0 when it does not, and 2 on a usage error. Pass a GNU `expand` to print GNU's output
next to uutils'.

```sh
cargo build --features unix
./repro.sh target/debug/coreutils /path/to/gnu-9.12/expand
```

Results seen (2026-10-16): uutils `main` 2e28cc30 → exit 1 (`2,/2` and `4,+2` fail, the `2,/3`
control passes); the same commit with `../fix.diff` → exit 0.

`harness-case/` is the original fuzzer case (argv `--tabs=2,/2`, empty stdin, GNU's recorded
behaviour). Run it with `harness-case/repro.sh <build-dir>`; `<build-dir>/bin/expand` must exist.
