# Repro

- `repro.sh <binary>`: `<binary>` is a uutils `sort`, a uutils multicall `coreutils`, or GNU `sort`.
  It runs 7 small cases under `LC_ALL=C` and compares each with the output recorded from GNU coreutils 9.12.
  Exit 1 = the bug reproduces (some case differs), 0 = every case matches, 2 = usage error.
  - Example: `./repro.sh ~/coreutils/target/debug/coreutils`
- `finding-repro/`: the original fuzzer case (`sort --sort=numeric`, stdin `b\n\f2`) with GNU's recorded output.
  Run it from its own directory: `./repro.sh <build-dir>`, where `<build-dir>/bin/sort` is the uutils binary.
  It exits 1 when the bug reproduces.

Results on 2026-10-16: upstream main 2e28cc30 -> exit 1 (0/7 cases match); main + fix.diff -> exit 0;
GNU 9.12 -> exit 0.
