# turso-diff

Differential testing of [Turso](https://github.com/tursodatabase/turso), the
Rust rewrite of SQLite, against SQLite 3.50.4, the version Turso tracks
(its COMPAT.md: an undocumented deviation from that version is a bug). The
inputs are [ephyra-sqlite](../ephyra-sqlite)'s, through the pinned copy
[ephyra-sqlite-pinned](../ephyra-sqlite-pinned): a generated database
file, statements over it, and queries over the final schema. Bugs found:
[bugs/turso](../bugs/turso).

## How it works

[src/main.rs](src/main.rs) runs `--jobs` workers for `--minutes`, each
drawing inputs from `case_queries` by seed. For each input both engines
open the same file (SQLite deserialized in memory, Turso from a copy on
disk through its Rust API), run the statements, then each query, and the
results are compared:

- rows in order when the query's ORDER BY fixes it, else as multisets;
  values exactly;
- text as UTF-8 with U+FFFD for invalid bytes, Turso's documented behavior;
- a BLOB that is valid JSONB by its JSON text, since one JSON value has
  several JSONB encodings.

The file is first made into one Turso can open as SQLite would: an
auto-vacuum database rewritten without auto-vacuum, a UTF-16 one copied to
UTF-8, indexes on WITHOUT ROWID tables dropped (a known Turso issue, and
Turso cannot create them).

Not differences, counted by reason: a case SQLite 3.50.4 rejects (the
generator follows SQLite trunk), a statement or query Turso rejects as
unsupported, and a difference where SQLite trunk gives what Turso gave (a
3.50.4 behavior newer SQLite changed). Trunk runs in a separate program,
[trunk-oracle](trunk-oracle/src/main.rs), since two SQLite versions cannot
be linked into one binary; without it built, that filter is off.

Findings: a Turso panic, a query only one engine fails, rows that differ, a
statement only Turso fails, and a worker stuck on one input for 60 s. Each
is grouped by a signature (kind and normalized message), kept up to three
examples per signature, and shrunk with the generator's shrinker to an
input that still gives the same message. A finding that narrow rules
attribute to an already filed Turso issue (`known_rules`) keeps one
unshrunk example per issue; issues fixed on the Turso commit under test are
listed in `FIXED` and not attributed, so a regression shows as new.

Output under `--out DIR`: `stats.txt` (inputs, compared queries, skips and
findings by signature, rewritten every 30 s), and per example
`findings/<signature>/<n>/` with the database (`case.db`), the statements,
the queries, a `replay.sql`, and their shrunk forms; `findings/known/<issue>/`
and `findings/hang/w<worker>/`.

## Setup

1. Run `case-studies/vendor/fetch.sh`. turso-diff patches in SQLite 3.50.4
   (`libsqlite3-sys-3.50.4`), which the generator's own rusqlite then links
   too; trunk-oracle patches in trunk (`libsqlite3-sys-trunk-bfe473ab`).
2. Clone Turso at the repository root (`turso/`, ignored by git); the
   dependency is `turso = { path = "../../turso/bindings/rust" }`. The
   revision last used is `45a0935a73d20808d12958cbd3ed9d001189b233` on
   `main` (2026-10-15, Turso 0.9.0-pre.1):

   ```bash
   git clone https://github.com/tursodatabase/turso.git
   git -C turso checkout 45a0935a73d20808d12958cbd3ed9d001189b233
   ```

   On a newer Turso, update `FIXED` and the rules in `known_rules` by
   rerunning each issue's reproduction.
3. Build trunk-oracle (optional; turso-diff looks for it at
   `trunk-oracle/target/release/trunk-oracle`):

   ```bash
   (cd trunk-oracle && cargo build --release)
   ```

## Run

```bash
cargo run --release -- --out runs/1 --seed 1 --minutes 30 --jobs 8
cargo run --release -- --out runs/1 --experimental     # also enable Turso's experimental WITHOUT ROWID, generated columns, ATTACH
cargo run --release -- --replay case.db replay.sql      # each statement on Turso and SQLite 3.50.4, both outputs printed
trunk-oracle/target/release/trunk-oracle case.db replay.sql   # the same on SQLite trunk
```

A 4-hour run with 14 workers drew 2.72 million inputs and compared 335,000
queries.

## What it found

39 Turso issues filed from its findings, each with a folder under
[bugs/turso](../bugs/turso) holding the report and its reproduction; the
index [bugs/README.md](../bugs/README.md) gives each one's status.

| Issue | Folder |
|---|---|
| [#9836](https://github.com/tursodatabase/turso/issues/9836) | [turso-json-arrow-same-operand-panic](../bugs/turso/turso-json-arrow-same-operand-panic/) |
| [#9837](https://github.com/tursodatabase/turso/issues/9837) | [turso-date-modifier-trailing-text](../bugs/turso/turso-date-modifier-trailing-text/) |
| [#9838](https://github.com/tursodatabase/turso/issues/9838) | [turso-json-each-null-path](../bugs/turso/turso-json-each-null-path/) |
| [#9839](https://github.com/tursodatabase/turso/issues/9839) | [turso-json-each-large-integer](../bugs/turso/turso-json-each-large-integer/) |
| [#9840](https://github.com/tursodatabase/turso/issues/9840) | [turso-json-negative-zero](../bugs/turso/turso-json-negative-zero/) |
| [#9841](https://github.com/tursodatabase/turso/issues/9841) | [turso-json-error-position-trailing-whitespace](../bugs/turso/turso-json-error-position-trailing-whitespace/) |
| [#9842](https://github.com/tursodatabase/turso/issues/9842) | [turso-unused-cte-eager-resolve](../bugs/turso/turso-unused-cte-eager-resolve/) |
| [#9843](https://github.com/tursodatabase/turso/issues/9843) | [turso-json-patch-repeated-key](../bugs/turso/turso-json-patch-repeated-key/) |
| [#9844](https://github.com/tursodatabase/turso/issues/9844) | [turso-json-set-null-handling](../bugs/turso/turso-json-set-null-handling/) |
| [#9845](https://github.com/tursodatabase/turso/issues/9845) | [turso-json-arrow-abbreviated-path](../bugs/turso/turso-json-arrow-abbreviated-path/) |
| [#9847](https://github.com/tursodatabase/turso/issues/9847) | [turso-right-join-vtab-subquery-duplicate-cursor](../bugs/turso/turso-right-join-vtab-subquery-duplicate-cursor/) |
| [#9848](https://github.com/tursodatabase/turso/issues/9848) | [turso-materialized-cte-subquery-before-loop](../bugs/turso/turso-materialized-cte-subquery-before-loop/) |
| [#9849](https://github.com/tursodatabase/turso/issues/9849) | [turso-real-affinity-parse](../bugs/turso/turso-real-affinity-parse/) |
| [#9850](https://github.com/tursodatabase/turso/issues/9850) | [turso-right-join-on-subquery-unmatched-rows](../bugs/turso/turso-right-join-on-subquery-unmatched-rows/) |
| [#9851](https://github.com/tursodatabase/turso/issues/9851) | [turso-subquery-hoisted-eval](../bugs/turso/turso-subquery-hoisted-eval/) |
| [#9852](https://github.com/tursodatabase/turso/issues/9852) | [turso-right-join-without-rowid-secondary-index](../bugs/turso/turso-right-join-without-rowid-secondary-index/) |
| [#9880](https://github.com/tursodatabase/turso/issues/9880) | [turso-having-group-term-not-pushed](../bugs/turso/turso-having-group-term-not-pushed/) |
| [#9881](https://github.com/tursodatabase/turso/issues/9881) | [turso-hash-join-real-affinity](../bugs/turso/turso-hash-join-real-affinity/) |
| [#9882](https://github.com/tursodatabase/turso/issues/9882) | [turso-materialized-cte-first-filled-in-subquery-loop](../bugs/turso/turso-materialized-cte-first-filled-in-subquery-loop/) |
| [#9883](https://github.com/tursodatabase/turso/issues/9883) | [turso-window-minmax-order-by-table-not-found](../bugs/turso/turso-window-minmax-order-by-table-not-found/) |
| [#9884](https://github.com/tursodatabase/turso/issues/9884) | [turso-strftime-out-of-range](../bugs/turso/turso-strftime-out-of-range/) |
| [#9885](https://github.com/tursodatabase/turso/issues/9885) | [turso-json-pretty-empty-container](../bugs/turso/turso-json-pretty-empty-container/) |
| [#9886](https://github.com/tursodatabase/turso/issues/9886) | [turso-timediff-day-overflow](../bugs/turso/turso-timediff-day-overflow/) |
| [#9887](https://github.com/tursodatabase/turso/issues/9887) | [turso-having-group-term-predicate](../bugs/turso/turso-having-group-term-predicate/) |
| [#9888](https://github.com/tursodatabase/turso/issues/9888) | [turso-strftime-real-formatting](../bugs/turso/turso-strftime-real-formatting/) |
| [#9889](https://github.com/tursodatabase/turso/issues/9889) | [turso-real-column-negative-zero](../bugs/turso/turso-real-column-negative-zero/) |
| [#9921](https://github.com/tursodatabase/turso/issues/9921) | [turso-date-hhmm-modifier-whole-days](../bugs/turso/turso-date-hhmm-modifier-whole-days/) |
| [#9924](https://github.com/tursodatabase/turso/issues/9924) | [turso-json-insert-root-path](../bugs/turso/turso-json-insert-root-path/) |
| [#9925](https://github.com/tursodatabase/turso/issues/9925) | [turso-minint-literal-order-by-equivalence](../bugs/turso/turso-minint-literal-order-by-equivalence/) |
| [#9926](https://github.com/tursodatabase/turso/issues/9926) | [turso-partial-index-between-covering-panic](../bugs/turso/turso-partial-index-between-covering-panic/) |
| [#9927](https://github.com/tursodatabase/turso/issues/9927) | [turso-json-infinity-rendering](../bugs/turso/turso-json-infinity-rendering/) |
| [#9932](https://github.com/tursodatabase/turso/issues/9932) | [turso-update-partial-index-rowid-unpositioned-insert](../bugs/turso/turso-update-partial-index-rowid-unpositioned-insert/) |
| [#9933](https://github.com/tursodatabase/turso/issues/9933) | [turso-update-before-trigger-real-affinity-index](../bugs/turso/turso-update-before-trigger-real-affinity-index/) |
| [#9934](https://github.com/tursodatabase/turso/issues/9934) | [turso-partial-index-join-covering-panic](../bugs/turso/turso-partial-index-join-covering-panic/) |
| [#9935](https://github.com/tursodatabase/turso/issues/9935) | [turso-date-embedded-nul](../bugs/turso/turso-date-embedded-nul/) |
| [#9966](https://github.com/tursodatabase/turso/issues/9966) | [turso-group-by-expression-json-subtype](../bugs/turso/turso-group-by-expression-json-subtype/) |
| [#9967](https://github.com/tursodatabase/turso/issues/9967) | [turso-json-cache-integer-real-key](../bugs/turso/turso-json-cache-integer-real-key/) |
| [#9968](https://github.com/tursodatabase/turso/issues/9968) | [turso-date-vertical-tab](../bugs/turso/turso-date-vertical-tab/) |
| [#9969](https://github.com/tursodatabase/turso/issues/9969) | [turso-strftime-day-overflow](../bugs/turso/turso-strftime-day-overflow/) |

Other folders under [bugs/turso](../bugs/turso) add what it found to issues
others filed before: #6089, #6749, #6864, #7273, #8221, #8322, #8452, #8538,
#9073, #9081.
