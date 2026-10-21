# repro

- `repro.sh [csplit command...]`: standalone. It runs the two cases from the report in a temp dir.
  Exit 1 = bug reproduces, 0 = matches GNU coreutils 9.12, 2 = could not run.
  Examples: `./repro.sh ./target/debug/coreutils csplit`, `./repro.sh csplit` (GNU, prints ok).
- `harness-case/`: the fuzzer's original minimal case. `harness-case/repro.sh <build-dir>` expects
  `<build-dir>/bin/csplit`. It compares stdout with `harness-case/expected/` (recorded from GNU 9.12).
