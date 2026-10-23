# sqlite-coverage

The SQLite code that [ephyra-sqlite](../generator/)'s generated cases and
oracle inputs reach, read in-process from an instrumented SQLite: the
amalgamation is compiled by clang with `-fprofile-instr-generate
-fcoverage-mapping` (`.cargo/config.toml`), Rust code is not instrumented, so
the counters are SQLite's alone. On it: frozen scores of the generators, the
experiments that chose the fuzzing loops, and the unattended coverage-guided
fuzz runner.

SQLite is trunk in place of libsqlite3-sys's bundled release, through
`[patch.crates-io]` to `../vendor/libsqlite3-sys-trunk`, which is not in the
repository: run `case-studies/sqlite/vendor/fetch.sh` first. Depends on
`../generator` and the library at `../../../crates/ephyra`. Needs clang and
its profile runtime (on Ubuntu, `libclang-rt-<version>-dev`).

```bash
case-studies/sqlite/vendor/fetch.sh
cd case-studies/sqlite/coverage
cargo build --release
target/release/score --seed 0 --cases 10000   # about 325
```

| Binary | What it does |
| --- | --- |
| `score` | The case generator's score: the file-format code 10 cases are expected to reach beyond their controls (the same rows written by SQLite). |
| `qscore` | The query-coverage score of the oracle inputs: the in-scope SQLite code 10 inputs are expected to reach under the oracles. Frozen; generator rounds are measured by it. |
| `heritability` | Do mutants of an input that reached a rare counter reach it again more often than fresh draws? |
| `targeted` | FairFuzz-style targeting of rare counters against fresh generation, at equal wall-clock time. |
| `fuzz` | Does mutation pay: fresh draws against ephyra's pool and a mix, at equal wall-clock time. |
| `gapcov` | A probe of one gap (type affinity and collation) over `qscore`'s inputs. |
| `runner` | The fuzz runner: a supervisor and worker processes for runs of days, findings shrunk and recorded, metrics over DogStatsD. |

`sections.tsv` maps each amalgamation function to its source file
(`deploy/sections.py` regenerates it). Running the runner unattended, the
nightly move to SQLite trunk, and the pre-triage of findings against the
known bugs of [`../bugs/`](../bugs/): [deploy/README.md](deploy/README.md).
