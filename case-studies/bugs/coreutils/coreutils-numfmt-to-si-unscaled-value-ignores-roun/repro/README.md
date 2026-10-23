# repro

- `repro.sh <numfmt>`: standalone check against any numfmt binary (`target/debug/numfmt`, or
  `"target/release/coreutils numfmt"`). Expected values are GNU coreutils 9.12 output. Exit 1 = bug
  reproduces, 0 = matches GNU.
  Verified: uutils main 2e28cc304 → exit 1 (8 FAIL). Same tree + `../fix.diff` → exit 0. GNU 9.12 → exit 0.
- `harness-minimal/`: the fuzzer's minimal case (`echo 12.1 | numfmt --to=si`, GNU output in `expected/`).
  Run `harness-minimal/repro.sh <dir with bin/numfmt>`. Exit 1 = reproduces.
