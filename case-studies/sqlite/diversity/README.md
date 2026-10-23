# diversity-probe

Measurements of the inputs [ephyra-sqlite](../generator)'s generator
draws, by the SQLite code they reach: how diverse they are, and where an
input's time goes. Used to decide how the coverage-guided runner in
[sqlite-coverage](../coverage) should draw (how many queries per
database, how often to mutate). It finds no bugs itself.

SQLite is compiled by clang with coverage instrumentation
([.cargo/config.toml](.cargo/config.toml)); the Rust code is not
instrumented, so the counters ephyra reads in-process
(`ephyra::coverage`, with the `coverage-force` feature since the build
script cannot see C flags) are SQLite's alone.

| Program | Measures |
|---|---|
| [`diversity-probe`](src/main.rs) | 20000 fresh draws seeded as the runner's workers seed them, each under one of the seven query oracles in turn: repeated inputs by size, distinct coverage fingerprints (the set of SQLite counters an input reaches), counters per input, the Jaccard similarity of random pairs, and the curve of distinct counters against inputs |
| [`disco`](src/bin/disco.rs) | coverage found per second with k = 3, 30, 300 queries per database, and k log-uniform in 1..300: distinct counters after 30, 60 and 120 s |
| [`ratio`](src/bin/ratio.rs) | time per input split into drawing the case, drawing its queries, loading the database and running the oracle; then, for k queries on the same database, time per query under each oracle |

## Build and run

Run `case-studies/sqlite/vendor/fetch.sh` first (the crate patches in SQLite trunk
from `case-studies/sqlite/vendor/libsqlite3-sys-trunk`). clang must be installed:
it compiles SQLite and links the profile runtime; on Linux it is also the
linker (Ubuntu: `clang` and `libclang-rt-<version>-dev`).

```bash
cargo run --release                 # diversity of 20000 draws
cargo run --release --bin disco     # about 8 minutes: four settings of 120 s
cargo run --release --bin ratio
```

Profile files go to `target/profraw/`.
