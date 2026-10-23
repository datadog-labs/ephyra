Build brush with overflow checks (the default dev profile does this):

    cargo build -p brush-shell
    ./repro.sh target/debug/brush

Exit 1 = bug reproduces (panic, exit 101). Exit 0 = output matches bash 5.3 (`9223372036854775807`).
