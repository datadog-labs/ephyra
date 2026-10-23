- `repro.sh [csplit command...]`: runs `seq 3 | csplit - /2/ 2 /3/-1` with a 10 s limit. Exit 1 = hangs
  (bug), 0 = GNU 9.12 output (`2 0 0 4`, exit 0), 2 = something else.
  Example: `./repro.sh ./target/debug/coreutils csplit`.
- `harness-case/`: the fuzzer's minimal case (`printf '1\n' > f1; csplit f1 /1/ 1 /x/-1`).
  `harness-case/repro.sh <build-dir>` needs `<build-dir>/bin/csplit`. Exit 1 = hangs.
