# turso-apidiff

An API-interleaving differential of [Turso](https://github.com/tursodatabase/turso)
(`turso_core`) against SQLite 3.50.4. Where [turso-diff](../differential)
compares SQL scripts, turso-apidiff drives both engines through their
statement APIs in patterns a script never produces: several prepared
statements on one connection stepped in turn, one left suspended partway
while another runs transaction control, a write or DDL; RETURNING statements
stepped partway; statements reset or finalized early; a second connection on
the same file; application functions and a writable application virtual
table registered identically in both engines. Bugs found:
[bugs/turso](../bugs) (table below).

## How it works

A session ([src/gen/mod.rs](src/gen/mod.rs)) is a schema and data, then API actions
over 2-5 prepared-statement slots on one connection (two connections on the
same file in about a third of the sessions, WAL and busy timeout 0 in both
engines):

- prepare (finalizing what the slot held), bind (fresh or reset statements
  only), step k times (1, 2-3 or to the end; auto-reset after done or an
  error, as `sqlite3_step` does), reset, finalize;
- one-shot statements: BEGIN [DEFERRED|IMMEDIATE|EXCLUSIVE], COMMIT, END,
  ROLLBACK, SAVEPOINT, RELEASE, ROLLBACK TO, DML (INSERT/UPDATE/DELETE/
  REPLACE/UPSERT with every conflict clause, RETURNING half of the time),
  DDL (CREATE/DROP TABLE/INDEX IF [NOT] EXISTS, ALTER TABLE ADD COLUMN).
  Transaction control and DML also run as prepared statements in slots.

The generator weights transaction control, writes and DDL up while a
statement is suspended. Schemas have 1-3 tables (INTEGER PRIMARY KEY
[AUTOINCREMENT], typed and untyped columns, UNIQUE/NOT NULL/CHECK/COLLATE
NOCASE, partial and expression indexes), triggers and foreign keys with ON
DELETE actions.

Both engines run the session in lockstep ([src/engines.rs](src/engines.rs)):
SQLite through raw `libsqlite3-sys` calls (`prepare_v2`, `step`, `reset`,
`finalize`, `bind`, `column_value`, `changes64`, `total_changes64`,
`last_insert_rowid`, `get_autocommit`), Turso through `turso_core`
(`Database::open`, `Connection::prepare`, `Statement::step/reset/bind_at`,
with `StepResult::IO/Busy` handled), which gives per-step control, Busy as a
step result and several live statements per connection. Compared after every
action: the result class of each prepare/bind/step/reset (not messages), the
rows (RETURNING rows as a multiset once the statement ends), and per
connection `changes()`, `total_changes()`, `last_insert_rowid()` and
autocommit. At the end: finalize everything, dump every table, run
integrity_check, commit open transactions, checkpoint Turso's WAL, close,
reopen both files with SQLite (Turso's file read by SQLite) and compare
again. The first divergence ends a session.

Application functions, the same in both engines: `app_echo`, `app_text`,
`app_type`, `app_concat` and the aggregates `app_count`, `app_isum`
(SQLite: `sqlite3_create_function_v2`; Turso: its native extension API,
`ExternalFunc::new_native_scalar` / `new_native_aggregate`).

### Generators

`--gen 1|2|3` selects the generator (default 3). They live in `src/gen/`:
`mod.rs` holds what they share (the action language, value pools and session
scripts), `v1.rs` is round 1 and also the building blocks (schema, reads,
writes, transaction control, DDL) that `v2.rs` and `v3.rs` extend. Each round
kept the earlier ones unchanged, as the baselines for coverage comparisons.

- Round 1 ([src/gen/v1.rs](src/gen/v1.rs)): the above, plus `json_each`/`json_tree`.
- Round 2 ([src/gen/v2.rs](src/gen/v2.rs)): a writable application virtual table
  `appvt` ([src/vt.rs](src/vt.rs)), one storage logic behind both engines:
  SQLite through an iVersion-1 `sqlite3_module` (xBegin/xSync/xCommit/
  xRollback), Turso through `OpenOptions::native_module` (`VirtualTable`,
  begin/commit/rollback). Writes fail deterministically on chosen values,
  before or after writing; every hook call is logged in traces. ATTACH
  (Turso's `with_attach`), DDL from the other connection while statements
  are prepared (re-prepare), window and aggregate reads, and a bulk insert
  in the setup so sessions split pages.
- Round 3 ([src/gen/v3.rs](src/gen/v3.rs)): a second attached database, TEMP
  tables and triggers, DETACH/ATTACH while statements are suspended and
  inside transactions, `PRAGMA [db.]wal_checkpoint(MODE)` (rows not
  compared: frame counts are per engine), cross-database statements, vtab
  RENAME and DROP inside transactions, and MVCC sessions: Turso opens the
  file with `with_mvcc(true)`, SQLite stays in WAL, compared only where both
  promise the same thing (one connection, with `BEGIN CONCURRENT` sent to
  SQLite as `BEGIN DEFERRED`; or two connections with plain BEGIN only, lock
  outcomes not compared). MVCC sessions use AUTOINCREMENT tables, since
  Turso's MVCC never reuses a plain rowid (#7474), and send `OR FAIL` as
  `OR ABORT` (gaps in an AUTOINCREMENT sequence are permitted).

### Classification

Each divergence is classified ([src/main.rs](src/main.rs), `classify`):

- finding: a Turso panic or an unexplained difference; shrunk (greedy
  action and setup deletion, fewer steps, one connection) and written to
  `findings/<signature>/seed-N.txt` with the session as a script, the shrunk
  script and its trace.
- skip, counted by reason: a feature Turso rejects as unsupported
  (turso-diff's rule); COMPAT.md's documented same-connection gaps (a second
  writer, SAVEPOINT, RELEASE or ROLLBACK TO while a statement is in progress
  return Busy; a write statement reset unfinished inside BEGIN makes the
  transaction rollback-only; MVCC's shared autocommit transaction); a vtab
  written by a trigger (Turso rejects it).
- unspecified, kept apart: rows of a statement that was suspended while the
  same connection wrote a table it reads (SQLite leaves that undefined);
  SQLite's `SQLITE_ABORT_ROLLBACK` of pending statements; DROP refused with
  `SQLITE_LOCKED` while a statement is active; error precedence between a
  schema error and Busy; lock outcomes of two MVCC connections, among others.
- known: a divergence after the pattern of an already filed bug, attributed
  by the pattern seen earlier in the session (letters as in the source:
  B = #11542, E = #11540, F = #11536, G = #11537, K and D = #11547), and the
  open issues #11394/#10825 (snapshot taken at SAVEPOINT or by an
  attached-only statement), #9323/#9364 (a failed statement ends the
  connection's other scans), #8698, #6775, #9327/#9619, #9285.
- tolerated, counted, the session continues: `changes()` after CREATE INDEX
  and vtab writes (#11547), `last_insert_rowid()` after a window query
  (#8698), an error Turso gives at prepare and SQLite at the first step,
  Turso's internal `__turso_internal_%` tables, Turso refusing a checkpoint
  while another statement of the connection is active.

## Setup

1. Run `case-studies/sqlite/vendor/fetch.sh` (SQLite 3.50.4,
   `libsqlite3-sys-3.50.4`, patched in as in turso-diff).
2. Clone Turso at the repository root (`turso/`, ignored by git). The
   dependencies are `turso_core`, `turso_sqlite` and `turso_ext` from
   `../../../turso/` (the same path from [../bugs](../bugs), whose
   Cargo.toml builds the bug reproductions). Tested at
   `2709799da` on `main` (2026-11-06, Turso 0.9.0-pre.2); round 1 and 2 ran
   on `52d6fd81d`:

   ```bash
   git clone https://github.com/tursodatabase/turso.git
   git -C turso checkout 2709799da
   ```

   turso-diff documents an older revision for the same `turso/` checkout;
   switch the checkout to build the other harness.

## Run

```bash
cargo run --release -- run --jobs 4 --minutes 10 --seed 1 --out runs/1
cargo run --release -- one SEED          # trace one session
cargo run --release -- replay FILE       # trace a session script (APIDIFF_CONTINUE=1: go on after a divergence)
cargo run --release -- shrink FILE       # shrink a session script
```

`runs/1/stats.txt` has sessions, actions by kind, the interleavings
exercised ("interleave: INSERT over suspended read", "left suspended:
writer RETURNING"), SQLite error classes, and skips, known, unspecified and
findings by signature. About 300 sessions/s on 4 jobs with generator 1,
30-70 with generators 2 and 3.

Coverage: build with `RUSTFLAGS="-C instrument-coverage"` into a separate
target directory, run with `APIDIFF_NOSHRINK=1`, merge with
`llvm-profdata` and read `llvm-cov export -summary-only` and
`llvm-cov report -show-functions` for the files under `turso/core`. Two
arms with the same binary (`--gen 2` against `--gen 3`, 5 minutes, 4 jobs
each) on 2709799da:

| file | lines | gen 2 | gen 3 |
|---|---|---|---|
| core/vdbe/execute.rs | 13,538 | 5,184 (38.3%) | 6,350 (46.9%) |
| core/connection.rs | 2,980 | 808 (27.1%) | 1,228 (41.2%) |
| core/storage/wal.rs | 3,593 | 1,500 (41.7%) | 1,701 (47.3%) |
| core/attach.rs | 468 | 160 (34.2%) | 225 (48.1%) |
| core/mvcc/database/mod.rs | 6,469 | 3 (0.0%) | 4,013 (62.0%) |
| all of core/ | 175,047 | 61,410 (35.1%) | 74,515 (42.6%) |

Generator 2 against generator 1 (on 52d6fd81d): 56,584 to 61,679 lines of
core/ (32.3% to 35.2%).

## Reproductions

Each bug folder under [../bugs](../bugs) is self-contained: the issue as
posted (`posted.md`), the reproduction as filed (`repro.rs`, the code block
of posted.md, or `repro.sql`), its SQLite 3.50.4 counterpart
(`repro_sqlite.rs` through rusqlite, the same `repro.sql` in the sqlite3
shell, or `repro-sqlite.sql` for the C virtual table below), fresh outputs
of both (`out-turso.txt`, `out-sqlite.txt`).
[../bugs/Cargo.toml](../bugs/Cargo.toml) (its own workspace) builds every
Rust reproduction, one binary per folder (`<folder>` and `<folder>-sqlite`);
its header has the build and run instructions for all of them, SQL ones
included. The virtual-table
reproductions use [../bugs/memvt](../bugs/memvt): `memvt`, a transactional
writable virtual table written with `turso_ext` (the documented extension
API), loaded with `.load` in tursodb, and its SQLite twin
[memvt_sqlite.c](../bugs/memvt/sqlite/memvt_sqlite.c); or Turso's own test
extension `kv_store`. The harness itself does not use them (its virtual
table is [src/vt.rs](src/vt.rs), in-process).

## What it found

Filed on tursodatabase/turso (2026-11-06), each with a folder under
[bugs/turso](../bugs) holding the issue as posted, its reproduction and
the outputs on both engines (Reproductions, above):

| Issue | Folder |
|---|---|
| [#11536](https://github.com/tursodatabase/turso/issues/11536) | [turso-noop-ddl-takes-write-lock](../bugs/turso-noop-ddl-takes-write-lock/) |
| [#11537](https://github.com/tursodatabase/turso/issues/11537) | [turso-rollback-to-schema-changed-error](../bugs/turso-rollback-to-schema-changed-error/) |
| [#11538](https://github.com/tursodatabase/turso/issues/11538) | [turso-attached-schema-change-fails-reader-transaction](../bugs/turso-attached-schema-change-fails-reader-transaction/) |
| [#11539](https://github.com/tursodatabase/turso/issues/11539) | [turso-attached-ddl-needs-main-snapshot](../bugs/turso-attached-ddl-needs-main-snapshot/) |
| [#11540](https://github.com/tursodatabase/turso/issues/11540) | [turso-reprepared-statement-discards-suspended-insert](../bugs/turso-reprepared-statement-discards-suspended-insert/) |
| [#11541](https://github.com/tursodatabase/turso/issues/11541) | [turso-vtab-rolled-back-by-failed-statement](../bugs/turso-vtab-rolled-back-by-failed-statement/) |
| [#11542](https://github.com/tursodatabase/turso/issues/11542) | [turso-reset-suspended-select-ends-explicit-transaction](../bugs/turso-reset-suspended-select-ends-explicit-transaction/) |
| [#11543](https://github.com/tursodatabase/turso/issues/11543) | [turso-window-select-reset-panics-with-suspended-writer](../bugs/turso-window-select-reset-panics-with-suspended-writer/) |
| [#11544](https://github.com/tursodatabase/turso/issues/11544) | [turso-vtab-other-connection-write-panics](../bugs/turso-vtab-other-connection-write-panics/) |
| [#11545](https://github.com/tursodatabase/turso/issues/11545) | [turso-vtab-update-rowid-panics](../bugs/turso-vtab-update-rowid-panics/) |
| [#11546](https://github.com/tursodatabase/turso/issues/11546) | [turso-attached-busy-retry-stays-busy](../bugs/turso-attached-busy-retry-stays-busy/) |
| [#11547](https://github.com/tursodatabase/turso/issues/11547) | [turso-changes-counters](../bugs/turso-changes-counters/) (with [turso-create-index-resets-changes](../bugs/turso-create-index-resets-changes/), [turso-total-changes-or-fail-in-transaction](../bugs/turso-total-changes-or-fail-in-transaction/), [turso-vtab-writes-not-counted-in-changes](../bugs/turso-vtab-writes-not-counted-in-changes/)) |

Already filed by others: [turso-create-virtual-table-if-not-exists-fails](../bugs/turso-create-virtual-table-if-not-exists-fails/)
(#11352), and instances of #11394, #10825, #9323, #9364, #8698, #6775,
#9327, #9619, #9285 and #7474.
