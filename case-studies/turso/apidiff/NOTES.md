# turso-apidiff notes (append only)

## 2026-11-05 — round 1: design

Goal: Turso bugs reachable only through public-API patterns that SQL scripts never produce (several statements on one connection stepped in turn, one suspended across another's transaction control or write; RETURNING stepped partway; application functions), the patterns behind the SQLite bugs of the VDBE-direct campaign (runs/vdbe-20261102/NOTES.md in ephyra-sql).

Targets
- Turso latest main 52d6fd81d (2026-11-05) in a separate worktree, a separate worktree (`git worktree add --detach ... origin/main` from ~/Projects/turso), so the checkouts turso-diff uses stay put.
- SQLite 3.50.4: vendored libsqlite3-sys-3.50.4 (`[patch.crates-io]`, like turso-diff), driven through raw `rusqlite::ffi` calls (prepare_v2/step/reset/finalize/bind/column_value/changes64/total_changes64/last_insert_rowid/get_autocommit).

Driver choice: `turso_core` directly (Database::open with PlatformIO + OpenOptions(SqliteDialect), Connection::prepare, Statement::step/reset/bind_at, StepResult incl. IO/Busy/Yield), not the `turso` crate: it gives per-step control, Busy as a step result, several live statements per connection, and the native extension API for application functions (`ExternalFunc::new_native_scalar` / `new_native_aggregate`, all pub). The C layer (bindings/c) was not needed. Repros for maintainers use the public `turso` crate (repro/).

API features covered
- N statement slots (2..5) per session on one connection, 30% of sessions with a second connection on the same file (WAL in both engines; busy timeout 0 in both).
- Actions: prepare (finalizes what the slot held), bind (only on fresh/reset statements, only existing parameters), step k times (1, 2-3, or to the end; auto-reset after done/error as sqlite3_step does), reset, finalize, and one-shot statements (prepare + step to end + finalize): BEGIN [DEFERRED|IMMEDIATE|EXCLUSIVE], COMMIT, END, ROLLBACK, SAVEPOINT, RELEASE, ROLLBACK TO, DML (INSERT/UPDATE/DELETE/REPLACE/UPSERT, OR ROLLBACK/ABORT/FAIL/IGNORE/REPLACE, RETURNING half the time), DDL (CREATE/DROP INDEX/TABLE IF [NOT] EXISTS, ALTER TABLE ADD COLUMN). Transaction control and DML also run as prepared statements in slots. The generator weights transaction control and writes up while a statement is suspended, and finishes a suspended writer more often (to limit the documented same-connection BUSY).
- Schema: 1-3 tables (INTEGER PRIMARY KEY [AUTOINCREMENT], typed/untyped columns, UNIQUE/NOT NULL/CHECK/COLLATE NOCASE, indexes incl. partial and expression `lower(b)`), triggers (25%), foreign keys with ON DELETE actions (20%, PRAGMA foreign_keys=ON on connection 0).
- Application functions, both engines: app_echo(x) (sqlite3_result_value / clone), app_text(x) (sqlite3_value_text / Value::cast_text), app_type(x) (sqlite3_value_type), app_concat(x,y) (both as text), aggregates app_count(x), app_isum(x). Deterministic, used in SELECT lists, WHERE, GROUP BY, RETURNING, triggers.
- Virtual tables both support: json_each, json_tree (generate_series is not in the 3.50.4 library).

Unavailable in Turso (not generated): window functions from the native API (an extension aggregate "cannot be used as a window function"; the C layer's sqlite3_create_window_function and sqlite3_aggregate_context are stubs, COMPAT.md); extension functions in index expressions ("invalid expression in CREATE INDEX: app_echo(a)"); FTS3/4/5 and R-Tree; writable application virtual tables (Turso's native VirtualTableModule exists, the SQLite side would need a twin: next round); ATTACH (experimental flag: next round). Dropped from the oracle: app_after_text (read as text, return the value): SQLite documents that sqlite3_value_text may change the value's type, Turso's API has no such conversion, so it compares SQLite's implementation choice, not Turso.

Oracle, per action: result class of every prepare/bind/step/reset (messages ignored), rows (exact; RETURNING rows as multisets once the statement ends, whole-number REALs as integers there), and per connection changes(), total_changes(), last_insert_rowid(), autocommit. At the end: finalize all, list tables and dump each ORDER BY rowid, integrity_check, COMMIT open transactions, Turso wal_checkpoint(TRUNCATE), close, reopen both files with SQLite (Turso's file read by SQLite) and compare integrity_check + dump. The first divergence ends a session (APIDIFF_CONTINUE=1 in replay keeps going).

Classification ([classify] in src/main.rs)
- finding: Turso panic, or an unexplained difference (shrunk with greedy action/setup deletion, step-count and connection reduction; findings/<sig>/seed-N.txt with the session as a script, the shrunk script, the shrunk trace).
- skip (counted by reason): Turso "not supported"-style errors (turso-diff's rule); COMPAT.md's same-connection writer BUSY ("... SQL statements in progress", any SQLite result); COMPAT.md's rollback-only state after a write statement is reset/dropped unfinished inside a transaction (tainted session).
- unspecified (kept apart): rows of a statement that was suspended while the same connection wrote a table it reads (SQLite leaves it undefined), incl. SQLite CORRUPT there; SQLite's SQLITE_ABORT_ROLLBACK of pending statements on a rollback with schema changes; SQLite counting a statement that got SQLITE_BUSY as still in progress (refuses SAVEPOINT/COMMIT/RELEASE).
- known (attributed to a triaged candidate by the pattern seen earlier in the session, examples kept in known/): #11394 (SAVEPOINT started the transaction, 2 connections), B, E, A, F, K (below).
- tolerated (counted, session continues): changes() after CREATE INDEX (candidate D) and after a DML statement failed with SQLITE_BUSY or was reset/finalized unfinished (SQLite sets 0 when such a statement halts); last_insert_rowid() after a window query (#8698); a Turso prepare error that SQLite gives at the first step (SQLite prepares against its cached schema); internal `__turso_internal_%` tables in the table list (AUTOINCREMENT sequence backing tables, by design).

Usage: `cargo run --release -- run --jobs J --minutes M --seed S --out DIR`, `one SEED`, `replay FILE`, `shrink FILE`. Stats in DIR/stats.txt: sessions, actions by kind, "interleave: <op> over suspended read|writer RETURNING", "left suspended", sqlite error classes, skips/known/unspecified/findings by signature, tolerated counts. Throughput ~300 sessions/s on 4 jobs.

## 2026-11-05 — round 1: runs

All local, 4 jobs. (smoke1/2 were 1-2 minute shakedowns while the classification was being fixed.)

| run | seed | minutes | sessions | clean | finding | known | skip | unspecified |
|---|---|---|---|---|---|---|---|---|
| smoke3 | 100000 | 10 | 103,492 | 76,149 | 17,945 | (n/a) | 7,963 | 1,435 |
| smoke4 | 200000 | 10 | 176,682 | 130,317 | 13,754 | 17,139 | 13,729 | 1,743 |
| smoke5 | 300000 | 10 | 184,959 | 143,204 | 1,784 | 22,366 | 14,546 | 3,059 |

smoke5 detail: 55,656 sessions with 2 connections; 226,562 reads and 42,330 RETURNING writers left suspended; interleavings exercised in smoke3 (similar later): BEGIN/COMMIT/ROLLBACK/ROLLBACK TO/SAVEPOINT/RELEASE over a suspended read 10k-26k each, over a suspended RETURNING writer 0.9k-2.1k each; INSERT/UPDATE/DELETE over a suspended read 15k-33k. Skips: 13,699 documented same-connection writer BUSY, 847 documented rollback-only. Known by candidate: B 12,973, #11394 4,516, F 1,968, E 1,187, A 1,024, K 698. Findings left: panic (12 sessions), last_insert_rowid after a no-insert upsert following a window query (harness: the #8698 tolerance was cleared too early; fixed after the run), COMMIT/END while a statement is suspended (now attributed to A, see below), two-connection BUSY/done mixes downstream of A/#11394, SQLite LOCKED for DROP while a read is active (Turso allows; see #7728).

Probe sessions written during triage are not kept; their findings are summarized here and in the bug folders (each has found-by.txt, the session that found it).

## 2026-11-05 — round 1: triage

Probable Turso bugs, each reproduced on main 52d6fd81d with the `turso` crate (repro/, `cargo run --release --bin NAME`) against SQLite 3.50.4; folders in ../bugs/candidates/ with repro.rs, output.txt, session scripts, notes.md (facts + dedupe):
- A `turso-sibling-statement-end-stops-suspended-select`: a statement on the connection finishes (done, reset, or BEGIN;COMMIT) while a SELECT is suspended; Turso ends the connection's read transaction; after another connection commits and the connection reads again, the suspended SELECT returns done (rows lost, no error). Related open #9323/#9364/#9296 (other triggers of a silently ended scan).
- B `turso-reset-suspended-select-ends-explicit-transaction`: SELECT stepped, then BEGIN (or SAVEPOINT) + INSERT, then the SELECT is reset/dropped: Turso rolls the explicit transaction back (INSERT lost, COMMIT "no transaction is active"). Closest: closed #7260/#7345 (BEGIN before the SELECT, fixed).
- E `turso-reprepared-statement-discards-suspended-insert`: INSERT ... RETURNING stepped once, a statement prepared before a schema change is stepped (re-prepared), the INSERT finishes: its row is gone (silent data loss).
- panic `turso-window-select-reset-panics-with-suspended-writer`: window SELECT stepped, INSERT ... RETURNING stepped, reset the SELECT: `turso_assert` "shared autocommit transaction needed full rollback outside MVCC" (core/vdbe/mod.rs:3723).
- G `turso-rollback-to-schema-changed-error`: BEGIN; SAVEPOINT; prepare; other connection CREATE INDEX; ROLLBACK TO; step: "Database schema changed" (not re-prepared).
- F `turso-noop-ddl-takes-write-lock`: CREATE TABLE/INDEX IF NOT EXISTS (exists), DROP TABLE/INDEX IF EXISTS (missing) return BUSY while another connection writes.
- D `turso-create-index-resets-changes` (low): changes() 0 after CREATE INDEX.
- K `turso-total-changes-or-fail-in-transaction` (low): inside BEGIN, a failing INSERT OR FAIL's kept rows count in changes() but not total_changes().

Duplicates (not candidates): SAVEPOINT takes the snapshot at once: #11394 (open, handyman) and #10825; it explains `exec INSERT: sqlite done / turso Busy` after SAVEPOINT, "no such table" for a table another connection created, json_each over a row another connection replaced, SQLITE_SCHEMA after SAVEPOINT + ALTER on another connection. Window query overwrites last_insert_rowid(): #8698 (open).

Documented deviations (skip rules): same-connection second writer / SAVEPOINT / RELEASE / ROLLBACK TO return BUSY "SQL statements in progress"; rollback-only after an unfinished writer is reset/dropped inside BEGIN (COMPAT.md "Same-connection write statements").

Harness bugs fixed during triage: RETURNING rows compared per step (now multisets at end); signature granularity (conflict clause dropped); app_after_text and app_count-as-window removed (see design); expression index on an app function removed; Turso's internal sequence tables excluded from the table list; prepare-vs-step error timing; the window last_insert_rowid tolerance clearing.

Not Turso (for the SQLite side; not pursued here, forum not searched):
- SQLite data loss on 3.50.4 and trunk bfe473ab (3.55.0, 2026-10-13): in autocommit mode, statement X (INSERT ... SELECT) gets SQLITE_BUSY and is left unreset; another INSERT on the same connection returns SQLITE_DONE (it cannot commit while X counts as active); sqlite3_reset(X) then rolls the whole transaction back and the INSERT is gone (count 1 -> 0 on both connections).. Turso keeps the row.
- SQLite "no such table" on 3.50.4, 3.54.0 and trunk-new (3.55.0): on a connection that has not loaded the schema yet, `SAVEPOINT s0; BEGIN IMMEDIATE;` (fails: within a transaction) `ROLLBACK TO s0; SELECT * FROM t0;` fails with "no such table: t0" (CLI repro: `printf 'SAVEPOINT s0;\nBEGIN IMMEDIATE;\nROLLBACK TO s0;\nSELECT * FROM t0;\n' | sqlite3 db-with-t0`).
- SQLite returns an INTEGER for a REAL column passed through a function in RETURNING (`INSERT INTO t(a REAL) VALUES (0) RETURNING coalesce(a, 1)` gives 0 while typeof() says real), MEM_IntReal; the harness normalizes it.
- SQLite SQLITE_CORRUPT when a GROUP BY scan over a UNIQUE index continues after DELETE ... RETURNING on the same connection removed rows it had not reached (46 sessions in smoke5); inside SQLite's "undefined" overlap area, kept as unspecified.

## Next round (suggested)
- Writable application virtual tables: Turso native VirtualTableModule + an identical SQLite module (rusqlite vtab or raw sqlite3_module), single-row writes inside explicit transactions and across savepoints (the third VDBE-campaign pattern).
- ATTACH (Turso experimental flag) with BUSY from the attached database; MVCC mode (BEGIN CONCURRENT has no SQLite twin, but plain MVCC sessions against SQLite's semantics, with COMPAT.md's MVCC same-connection gap as a skip).
- Re-weight the generator: fewer two-connection writes while A/B/#11394 are open (they dominate "known"), more re-prepare (DDL between prepare and step) and more window/aggregate statements interleaved with writers (the panic and E came from there).
- Coverage: build turso_core with coverage counters and judge rounds by joint coverage of core/vdbe (Program::abort, auto_txn_cleanup paths, reprepare) instead of finding counts.

## 2026-11-06 — round 2: design

Turso main unchanged since round 1 (`git fetch`: origin/main = 52d6fd81d). Built against a clean worktree at the same commit. SQLite 3.50.4 as in round 1.

New code (round-1 code paths unchanged; `--gen 1` runs the round-1 generator, kept verbatim as `gen::session_v1`, for the A/B):
- src/vt.rs: writable application virtual table `appvt`, one module logic (`VtStore`) behind both engines. SQLite: raw iVersion-1 `sqlite3_module` via `sqlite3_create_module_v2` (xBegin/xSync/xCommit/xRollback, no savepoint methods: the closest match to Turso's native `VirtualTable` trait, which has begin/commit/rollback only). Turso: `OpenOptions::native_module("appvt", VTabKind::VirtualTable, ..)` (`VirtualTableModule`/`VirtualTable`/`VirtualTableCursor`/`TableUpdate`; needs `turso_ext` for `ResultCode`). `CREATE VIRTUAL TABLE vtN USING appvt(vtN)` (Turso's `create()` gets no table name, so the argument names the storage). Columns (a, b), full scans in rowid order, cursor copies the rows at xFilter. Writes fail deterministically: b = 'boom' or an existing rowid before writing; a = 13 (INSERT/UPDATE) and a deleted row with a = 99 after writing ("write then fail"). xBegin copies the rows, xCommit drops the copy, xRollback restores it. Every hook call is logged per action in traces (`vt hooks (same|DIFFER)`), not compared.
- ATTACH: Turso opened with `DatabaseOpts::with_attach(true)`; each engine attaches its own second file (`$AUX` in session SQL is replaced per engine), created empty in WAL mode by SQLite before the session. Final checks dump aux tables and the vtabs, and the file comparison covers the aux files.
- src/gen2.rs (round-2 generator, default): vt0/vt1 in 45% of sessions (writes from connection 0 only, 40% of its writes when present: single-row INSERT/UPDATE/DELETE with conflict clauses, bound values biased to 13, rowid changes (rare once known), INSERT ... SELECT FROM vt into real tables, multi-row VALUES (rare once known), reads/joins/windows over vtabs, CREATE/DROP of vt9 as DDL); ATTACH aux.u0 in 25% (two connections in 60% of those, both attached; round-1 templates moved to aux.u0 35% of the time; DDL on aux); DDL weight 3 (vs 1) while a statement is prepared/running and from the other connection half the time (re-prepare); window/aggregate reads (row_number, lag/lead, frames, rank, GROUP BY/HAVING) a third of reads; reset/finalize weights 3/2 when two statements are running; a deterministic bulk insert (60-300 rows, text up to 700 chars) in 35% of setups (added after the first coverage A/B, below).
- Two-connection noise from #11394/#10825: Turso takes the snapshot at SAVEPOINT also after a deferred BEGIN (BEGIN; SAVEPOINT; other connection INSERT; INSERT -> Turso Busy stale, SQLite done), so "BEGIN first" does not help; instead, on two-connection sessions, 80% of BEGIN/SAVEPOINT get a read of main (and aux when attached) first so both engines hold the snapshot. Attribution widened: SAVEPOINT on a connection that has not read in the transaction (or not read aux), and transactions/suspended statements that read only the attached database (#10825).
- Harness: no-shrink switch `APIDIFF_NOSHRINK=1` (coverage runs); harness panics are printed (`harness panic: ...`).

Harness bugs fixed during round 2: `retarget` sliced UTF-8 by byte (worker threads panicked on 'é': smoke1 first attempts died with exit 101, ~11 sessions/s); RETURNING rows of a statement reset/finalized before done were compared as multisets (which rows come first is unspecified; now only the count); Turso's prepare-time schema error vs SQLite's BUSY at the first step for one-shot statements; DROP with an active statement on the same connection (SQLITE_LOCKED) now unspecified (#7728 area).

New classification (src/main.rs `classify`), in addition to round 1:
- known: window-select-reset panic; G-family (Turso "Database schema changed" on two connections, with or without ROLLBACK TO); #9323/#9364 (a failed statement, ROLLBACK or ROLLBACK TO with another statement suspended on the connection); F widened to a no-op or failed DDL earlier in the other connection's transaction; #6775 (multi-row INSERT into a vtab keeps only the last row); and the round-2 candidates below (vt-update-rowid panic, vt-other-connection, vt-rollback via the hook log: Turso rollback() without SQLite xRollback, aux-ddl-stale, aux-schema-changed, aux-busy-retry).
- tolerated: changes()/total_changes()/last_insert_rowid() after a vtab write (candidate turso-vtab-writes-not-counted-in-changes; total_changes offset carried per connection).
- unspecified: SQLite statement that got SQLITE_BUSY and was not reset (keeps its read transaction; another statement on that connection afterwards taints the session, also for the file comparison: round-1 SQLite note); last_insert_rowid() after a failed INSERT (FK + trigger); group key representative INTEGER vs whole REAL (`GROUP BY a` with 2 and 2.0); SQLite's schema error before Turso's BUSY; SQLite taking main's write lock for a vtab write.

## 2026-11-06 — round 2: runs

All local, 4 jobs, 10 minutes, generator v2 (smoke1/2 ran in parallel, smoke3/4 each in parallel with the two coverage runs; classification improved between runs, so later runs have fewer "finding" by construction).

| run | seed | sessions | clean | finding | known | skip | unspecified | 2 conns |
|---|---|---|---|---|---|---|---|---|
| r2-smoke1 | 500000 | 40,842 | 30,113 | 766 | 6,340 | 2,691 | 932 | 15,402 |
| r2-smoke2 | 600000 | 40,448 | 30,025 | 796 | 6,067 | 2,667 | 893 | 15,224 |
| r2-smoke3 | 700000 | 39,835 | 29,478 | 5 | 6,640 | 2,613 | 1,099 | 14,922 |
| r2-smoke4 | 900000 | 39,585 | 29,244 | 4 | 6,576 | 2,624 | 1,137 | 14,816 |

(r2-shake1, 3 minutes, was the shakedown with the UTF-8 harness bug.) Throughput ~67 sessions/s per run (two runs at once; v2 sessions cost ~2x v1's). smoke4: 44,382 reads and 6,178 RETURNING writers left suspended, 261k vtab hook calls, INSERT over a suspended read 14.0k, ROLLBACK TO over a suspended read 5.3k. smoke1/2 findings were 557/572 vt-update-rowid panics plus ~55 clusters, all triaged below; smoke3/4 leftovers: one SQLite snapshot-timing case each (below), the rest attributed after re-classification. Final binary, 1-minute check (seed 950000): 5,971 sessions, 0 findings.

## 2026-11-06 — round 2: coverage A/B

turso_core + harness built with `RUSTFLAGS="-C instrument-coverage"` and toolchain 1.99.1 (it ships llvm-tools; target dir target-cov), `APIDIFF_NOSHRINK=1`, 4 jobs, 5 minutes per arm, both arms at the same time (plus one smoke run), `llvm-profdata merge` + `llvm-cov export -summary-only`; line coverage of files under turso/core. v2 = generator before the bulk-insert change, v2b = final generator; v1/v1b = round-1 generator (two samples).

| file | lines | v1 | v1b | v2 | v2b |
|---|---|---|---|---|---|
| core/vdbe/execute.rs | 13,538 | 4,479 (33.1%) | 4,479 (33.1%) | 5,169 (38.2%) | 5,185 (38.3%) |
| core/vdbe/mod.rs | 2,383 | 1,273 (53.4%) | 1,282 (53.8%) | 1,324 (55.6%) | 1,335 (56.0%) |
| core/statement.rs | 956 | 379 (39.6%) | 400 (41.8%) | 399 (41.7%) | 402 (42.1%) |
| core/connection.rs | 2,980 | 653 (21.9%) | 653 (21.9%) | 782 (26.2%) | 787 (26.4%) |
| core/storage/pager.rs | 3,864 | 2,213 (57.3%) | 2,214 (57.3%) | 2,333 (60.4%) | 2,517 (65.1%) |
| core/storage/btree.rs | 5,925 | 3,736 (63.1%) | 3,709 (62.6%) | 2,895 (48.9%) | 4,449 (75.1%) |
| core/storage/wal.rs | 3,593 | 1,487 (41.4%) | 1,487 (41.4%) | 1,489 (41.4%) | 1,610 (44.8%) |
| core/vtab.rs | 435 | 120 (27.6%) | 120 (27.6%) | 194 (44.6%) | 194 (44.6%) |
| core/native_ext/vtab.rs | 114 | 0 | 0 | 85 (74.6%) | 85 (74.6%) |
| core/attach.rs | 468 | 0 | 0 | 160 (34.2%) | 160 (34.2%) |
| core/translate/schema.rs | 2,712 | 796 (29.4%) | 796 (29.4%) | 929 (34.3%) | 929 (34.3%) |
| core/translate/insert.rs | 2,452 | 1,815 (74.0%) | 1,815 (74.0%) | 1,880 (76.7%) | 1,880 (76.7%) |
| all of core/ | 175,049 | 56,584 (32.3%) | 56,613 (32.3%) | 59,081 (33.8%) | 61,679 (35.2%) |

Sessions per arm: v1 16,092, v2 13,055, v1b 22,169, v2b 15,491. v1 and v1b agree within 30 lines overall although v1b ran 38% more sessions: the round-1 generator had saturated. The first v2 lost 841 btree.rs lines: function-level diff (`llvm-cov report -show-functions`) showed `BTreeCursor::balance_non_root` (-454), `indexbtree_move_to_inner`/`_internal`, `page_insert_array`, `edit_page`, `defragment_page_fast`, `page_free_array`: v2 sessions held too few rows to split pages (vtab writes take real-table writes, sessions end earlier). The bulk setup insert fixed that (v2b btree.rs 75.1%). Net: v2b +5,066 lines in core (+9.0% relative), gains in every listed file; the vtab/attach code and execute.rs (+706, VUpdate/VBegin/VDestroy, attach, Halt paths) account for most of the rest. Data: runs/cov/{v1,v2,v1b,v2b}.json and *.profdata.

## 2026-11-06 — round 2: triage

New probable Turso bugs (folders in ../bugs/candidates/, each with repro.rs, output.txt, session.txt + session-trace.txt for the SQLite 3.50.4 side, notes.md facts + dedupe). Repros: apidiff/repro2 (new crate, turso_core with a minimal native module `mem` or the attach flag; `cargo run --release --bin NAME`):
- `turso-vtab-other-connection-write-panics` (vtab_other_connection): writing a native vtab from a connection other than its creator panics `Could not find virtual table for VBegin` (execute.rs:1731), then `vtab must exist` (execute.rs:4189) on drop; DROP TABLE from another connection fails "Could not find Virtual Table to Destroy". SQLite: both work.
- `turso-vtab-rolled-back-by-failed-statement` (vtab_failed_statement_rollback): any statement failing inside BEGIN/SAVEPOINT calls the module's rollback() (op_halt, execute.rs:3660 `if err_code > 0 { vtab_rollback_all }`) while the transaction continues and later commits: the vtab's earlier write is lost, the real tables keep theirs. SQLite: no xRollback.
- `turso-vtab-update-rowid-panics` (vtab_update_rowid_panic): `UPDATE v SET rowid = rowid + 1` panics "NotExists on unexpected cursor" (execute.rs:14270). Most frequent finding of smoke1/2.
- `turso-vtab-writes-not-counted-in-changes` (vtab_changes_rowid, low): vtab INSERT/UPDATE/DELETE not counted in changes()/total_changes(); INSERT does not set last_insert_rowid() (op_vupdate sets it only for OR REPLACE).
- `turso-create-virtual-table-if-not-exists-fails` (vtab_if_not_exists): `CREATE VIRTUAL TABLE IF NOT EXISTS v USING m` fails 'near "IF": syntax error' at step and creates nothing; `create_vtable_body_to_str` (translate/schema.rs:1963) writes `CREATE VIRTUAL TABLE v IF NOT EXISTS USING ...`.
- `turso-attached-ddl-needs-main-snapshot` (attached_ddl_stale): inside BEGIN, DDL on an attached database fails "Database snapshot is stale" when only main changed (an INSERT on the same attached table works), and holds main's write lock so the other connection's main writes get Busy. SQLite: both fine.
- `turso-attached-schema-change-fails-reader-transaction` (attached_schema_changed): a reader transaction gets "Database schema changed" after another connection changes an attached database's schema; for main it reads the snapshot. Possibly G's mechanism for attached databases.
- `turso-attached-busy-retry-stays-busy` (attached_busy_retry): a statement that got Busy on an attached database keeps returning Busy when stepped again after the lock holder committed (repro: until reset; harness session: one extra Busy); on main the first retry succeeds, as in SQLite.

Duplicates / attributed (not new candidates): multi-row `INSERT INTO vt VALUES (..), (..)` keeps only the last row = #6775 (open); a failed statement ending other open SELECTs on the connection (also single-connection, autocommit, any SELECT: `SELECT id FROM t1` stepped, failing INSERT into another table, SELECT returns done) = #9323 (open) / #9364; re-prepare of a statement inside a transaction after another connection's DDL (no SAVEPOINT needed: statement prepared before one schema change, transaction snapshot before a second one, step -> "Database schema changed") attributed to G as "G-family" (sessions in runs/r2-smoke*/findings/step-select-*-turso-schema); a failed DDL (duplicate column) inside BEGIN keeps the write lock in Turso like F's no-op DDL (attributed to F); SAVEPOINT snapshot after a deferred BEGIN = #11394 (see design).

Not candidates (documented in classification): SQLite LOCKED for DROP with a pending statement; SQLite error precedence (schema error before BUSY); SQLite taking the write lock for vtab writes (a Turso vtab write proceeds to the module while another connection holds main's lock; SQLite returns BUSY: the vtab is not tied to the pager in Turso, not a correctness issue for the module's own storage); GROUP BY representative 2 vs 2.0. An apparent changes() reset after CREATE VIRTUAL TABLE/ATTACH/DROP vtab was the vtab-changes candidate's after-effect.

Caveat: the "SQLite keeps the read transaction of an unreset SQLITE_BUSY statement" bucket is broad (37 shrunk examples in smoke1/2, ~200 sessions per run); it is checked before the known/A rules and may hide some A/#9323 instances.

Not Turso (SQLite-side observations, facts only, not pursued):
- SQLite 3.50.4 keeps the transaction of a statement that got SQLITE_BUSY and was not reset (as round 1): a later DDL/DML on that connection returns done but is rolled back when the BUSY statement is finalized (run r2-smoke2 seed 601140: `CREATE TABLE IF NOT EXISTS tmp1` missing from SQLite's file; r2-smoke1 seed 525936: `ALTER TABLE aux.u0 ADD COLUMN` not visible to the other connection).
- After a statement inside BEGIN fails and is rolled back completely (`DELETE ... RETURNING` failing a FOREIGN KEY check), COMMIT still makes another connection's older snapshot stale: its next write gets SQLITE_BUSY "database is locked" (r2-smoke3 seed in findings/exec-create-table-sqlite-busy-turso-done); Turso commits nothing and the other connection's write succeeds.
- `DROP TABLE IF EXISTS missing` inside BEGIN appears to start read transactions on attached databases too: a later write to aux after another connection wrote aux gets SQLITE_BUSY (r2-smoke4 findings/exec-insert-sqlite-busy-turso-done).
- An INSERT that fails with a FOREIGN KEY error at statement end (with an AFTER INSERT trigger) leaves last_insert_rowid() at the inserted row's rowid (Turso: unchanged).
- SQLite calls xCommit (no xBegin) on a virtual table at the end of the transaction that created it (`commit(no begin)` in traces).

## Next round (suggested, round 3)
- Coverage gaps to go after (llvm-cov on v2b): vtab writes from the second connection (now a known panic: keep a small weight once it is fixed or attributed), vtab xRename/ALTER TABLE RENAME, DROP of a vtab inside a transaction then ROLLBACK, savepoint hooks (an iVersion-2 SQLite twin is not possible against Turso's trait; compare only data with SAVEPOINT + failing statements), triggers on real tables reading vtabs (innocuous modules).
- ATTACH: DETACH while statements are suspended, two attached databases, temp database, ATTACH inside a transaction (error parity), checkpoint of aux (PRAGMA aux.wal_checkpoint) interleaved with readers.
- MVCC mode (still open from round 1).
- Keep judging by joint line coverage per file; check `-show-functions` diffs per round (the btree regression was only visible there).

## 2026-11-06 — round 3: design

Turso main moved to 2709799da (2026-11-06; `git ls-remote`; two commits since 52d6fd81d, a core/dialect/mod.rs refactor only). Built against a clean worktree at that commit. SQLite 3.50.4 as before. Panic-hook filter now matches any `turso-main*` path (it printed every Turso panic as a harness panic with the new path).

New generator src/gen3.rs (default; `--gen 2` keeps round 2 verbatim for the A/B; round-2 weights for re-prepare, window/aggregate reads and reset/finalize with two statements running are unchanged):
- ATTACH depth: attach probability 25% -> 35% (not in MVCC sessions); second attached database `aux2.u1` (`$AUX2`, replaced before `$AUX`) in 40% of attached sessions; TEMP table `w0` and TEMP triggers on t0 writing it (25%, connection 0); actions DETACH aux/aux2, ATTACH again (also inside transactions and while statements are suspended), DETACH+ATTACH pairs, checkpoints `PRAGMA [main.|aux.|aux2.]wal_checkpoint[(PASSIVE|FULL|RESTART|TRUNCATE)]` (also of main in unattached sessions); cross-database statements (`INSERT INTO aux.u0 SELECT ... FROM main.t0`, `INSERT INTO main.t SELECT FROM aux`, `UPDATE aux.u0 ... WHERE a IN (SELECT ... FROM main.t)`, aux <-> aux2 copies, joins, UNION ALL, NOT IN) as one-shots and in slots, weighted up while statements run.
- MVCC (12% of sessions, `-- mvcc 1` in scripts): Turso opens the file with `DatabaseOpts::with_mvcc(true)`, SQLite stays in WAL. Oracle choice: MVCC is compared against SQLite's WAL semantics only where both promise the same thing: (a) one connection: deferred/immediate transactions and BEGIN CONCURRENT (sent to SQLite as BEGIN DEFERRED: with no other connection a concurrent transaction must behave like a deferred one); (b) two connections with plain BEGIN/IMMEDIATE only (Turso's MVCC runs those as exclusive transactions, manual.md), where data and visibility must match but lock outcomes are only "unspecified" (see classification: MVCC upgrades fail on any commit since the snapshot, SQLite on any new WAL frame). A Turso MVCC-vs-WAL self-differential was not used: Turso WAL has the same rowid rule as SQLite and no BEGIN CONCURRENT, so it adds nothing over SQLite as the reference. Accommodations: MVCC sessions use AUTOINCREMENT tables, because Turso's MVCC never reuses a plain rowid (ignored/failed/rolled-back inserts, deleted maximum; open #7474), which SQLite's documented "usual" max+1 rule does not promise either; `OR FAIL` is sent as `OR ABORT` (a failed OR FAIL insert consumes an AUTOINCREMENT value in MVCC; gaps are permitted by autoinc.html); no virtual tables ("Virtual tables are not supported in MVCC mode"), no ATTACH, checkpoints TRUNCATE only (PASSIVE needs an experimental flag). At the end Turso's file is checkpointed and switched back with `PRAGMA journal_mode = wal` (SQLite cannot read an MVCC header); the results of these Turso-only statements are now in the trace (`final (turso only)`).
- Virtual tables: `ALTER TABLE vtN RENAME TO vt7|vt8` and back (both modules now log xRename / `VirtualTable::rename`), DROP of a vtab inside BEGIN or SAVEPOINT then ROLLBACK [TO] and a read, SAVEPOINT + a vtab write + a failing vtab write + ROLLBACK TO/RELEASE (the known vt-rollback path), a vtab scan stepped once, a write to the same vtab, the scan stepped to the end; triggers on t0 that read a vtab (or, 5%, write one). The module is innocuous on both sides now (`sqlite3_vtab_config(SQLITE_VTAB_INNOCUOUS)`, `innocuous() -> true`).

Harness changes: final checks list the attached databases from `PRAGMA database_list` (names compared) and dump each, dump temp tables, and list vtabs from sqlite_schema (they can be renamed/dropped); attached files are created when any action ATTACHes; `-log` files are removed; checkpoint rows are blanked (frame counts are per engine; only the result kind is compared); one-shot statements step to the end (100k cap: a bulk `DELETE ... RETURNING` returned >500 rows and was compared as a partial multiset: harness bug fixed).

New classification (round 3):
- tolerated: Turso refusing an explicit checkpoint while another statement of the connection is active (`begin_explicit_checkpoint`, by design; a checkpoint changes no data, the session continues).
- skip: "turso rejects a virtual table written by a trigger (unsafe use)" (translate/insert.rs:303 rejects any vtab write inside a trigger; SQLite allows innocuous vtabs); "documented: MVCC shared transaction" (an autocommit write finished while a sibling statement was active: COMPAT.md's MVCC gap; sticky, also for the file comparison); "MVCC: AUTOINCREMENT value kept after a rollback or failed statement in BEGIN CONCURRENT" (the id comes from an inner transaction that commits, #9505's mechanism; permitted gap).
- unspecified: checkpoint inside a transaction where the databases the transaction holds differ (SQLite's IF [NOT] EXISTS DDL verifies every attached schema, i.e. opens read transactions on all of them); DETACH refused by SQLite after such a DDL; SQLite BUSY after a RESTART/TRUNCATE checkpoint while another connection reads; MVCC two-connection lock outcome differences (both directions); nested BEGIN error precedence; WAL "snapshot is stale" in Turso where SQLite succeeds after another connection's UPDATE (SQLite's identical-content overwrite writes no frame; checked last, after all known rules).
- known (new candidates below): vt-registry (after a vtab RENAME or a DROP inside a transaction), ckpt-noop-stale, mvcc-commit-suspended, mvcc-concurrent-ddl; known existing: #9327/#9619 (MVCC panic "transaction should exist while its cursor is active"), #9285 (MVCC index keeps the old key, seen as SQLite integrity_check "row N values differ from index"), aux-busy-retry widened (Turso keeps an attached database's snapshot after Busy on it inside a transaction), #10825 widened to statements touching only aux/temp, the file comparison now attributes known patterns seen in the session (A, B, E, #9323, vt-*, aux-busy).

## 2026-11-06 — round 3: runs

All local, 10 minutes, 4 jobs, generator v3. smoke1 ran with the two 5-minute coverage arms, smoke2-4 together (with the second coverage pair for half of the time); classification improved between smoke1 and smoke2-4 (smoke1's 9 findings were then attributed: below).

| run | seed | sessions | clean | finding | known | skip | unspecified | 2 conns |
|---|---|---|---|---|---|---|---|---|
| r3-smoke1 | 5000000 | 30,482 | 20,458 | 9 | 6,957 | 2,041 | 1,017 | 12,180 |
| r3-smoke2 | 6000000 | 17,859 | 11,941 | 0 | 3,921 | 1,141 | 856 | 7,179 |
| r3-smoke3 | 7000000 | 17,628 | 11,770 | 2 | 3,895 | 1,134 | 827 | 7,034 |
| r3-smoke4 | 8000000 | 18,101 | 12,170 | 0 | 3,950 | 1,156 | 825 | 7,156 |

(r3-shake1..3 were 1-2 minute shakedowns while the harness and classification were fixed; r3-final-check, 2 minutes on 8 jobs, seed 9500000: 26,253 sessions, 3 findings, all attributed afterwards: a #9285 instance, an MVCC two-connection lock difference, an identical-overwrite staleness.) Throughput ~30-50 sessions/s per run. Totals over smoke1-4: 61,886 checkpoint statements (5,698 refused by Turso while another statement was active), DETACH over a suspended read 2,028, ATTACH over a suspended read 1,870, ALTER over a suspended read 5,016, 7,417 MVCC writes finished with a sibling active. New-candidate signatures over smoke1-4: vt-registry 6,738 (4,325 of them the VBegin/VRollback panics: the DROP+ROLLBACK action is frequent; lower its weight once fixed), ckpt-noop-stale 802, mvcc-commit-suspended ~515, mvcc-concurrent-ddl 68; #9327/#9619 panics 12; skips: MVCC shared transaction 952, AUTOINCREMENT gap 89, vtab written by trigger 48.

## 2026-11-06 — round 3: coverage A/B

Same method as round 2 (toolchain 1.99.1, `-C instrument-coverage`, target-cov, `APIDIFF_NOSHRINK=1`, 4 jobs, 5 minutes per arm, both arms at once, same binary: `--gen 2` vs `--gen 3`, turso_core at 2709799da; `llvm-cov export -summary-only` per file and `llvm-cov report -show-functions` per function, runs/cov3/covdiff.py). Note: the first attempt used `LLVM_PROFILE_FILE=...%c` (continuous mode) and produced empty profiles on macOS; plain `%p-%m` works. Two pairs: v2/v3 (seed 4000000, alongside smoke1) and v2b/v3b (seed 4100000, alongside smoke2-4).

| file | lines | v2 | v3 | diff |
|---|---|---|---|---|
| core/vdbe/execute.rs | 13,538 | 5,184 (38.3%) | 6,350 (46.9%) | +1,166 |
| core/vdbe/mod.rs | 2,383 | 1,335 (56.0%) | 1,420 (59.6%) | +85 |
| core/statement.rs | 956 | 402 (42.1%) | 431 (45.1%) | +29 |
| core/connection.rs | 2,980 | 808 (27.1%) | 1,228 (41.2%) | +420 |
| core/storage/pager.rs | 3,864 | 2,412 (62.4%) | 2,507 (64.9%) | +95 |
| core/storage/btree.rs | 5,925 | 4,447 (75.1%) | 4,355 (73.5%) | -92 |
| core/storage/wal.rs | 3,593 | 1,500 (41.7%) | 1,701 (47.3%) | +201 |
| core/vtab.rs | 435 | 194 (44.6%) | 198 (45.5%) | +4 |
| core/native_ext/vtab.rs | 114 | 82 (71.9%) | 88 (77.2%) | +6 |
| core/attach.rs | 468 | 160 (34.2%) | 225 (48.1%) | +65 |
| core/translate/alter.rs | 4,766 | 251 (5.3%) | 363 (7.6%) | +112 |
| core/translate/schema.rs | 2,712 | 929 (34.3%) | 947 (34.9%) | +18 |
| core/translate/insert.rs | 2,452 | 1,880 (76.7%) | 1,904 (77.7%) | +24 |
| core/translate/sequence.rs | 850 | 101 (11.9%) | 558 (65.6%) | +457 |
| core/mvcc/database/mod.rs | 6,469 | 3 (0.0%) | 4,013 (62.0%) | +4,010 |
| core/mvcc/database/checkpoint_state_machine.rs | 2,468 | 0 | 1,861 (75.4%) | +1,861 |
| core/mvcc/cursor.rs | 1,373 | 0 | 1,083 (78.9%) | +1,083 |
| core/skiplist/base.rs | 1,262 | 0 | 754 (59.7%) | +754 |
| core/mvcc/persistent_storage/logical_log.rs | 2,315 | 0 | 400 (17.3%) | +400 |
| all of core/ | 175,047 | 61,410 (35.1%) | 74,515 (42.6%) | +13,105 |

Sessions: v2 11,599, v3 9,802 (v3 sessions cost ~18% more). Second pair: v2b 61,476 vs v3b 74,491 (+13,015); same-generator noise: v2 vs v2b +66, v3 vs v3b -24 lines. So +13.0k lines (+21% relative), about 9k of it MVCC code (mvcc/, skiplist/, translate/sequence.rs), the rest execute.rs (+1,166: op_journal_mode_inner +162, op_vrename 0 -> 19 of 20 lines, op_checkpoint 46 -> 75 of 84 lines), connection.rs (+420), wal.rs (+201, restart/checkpoint paths), attach.rs (+65: `detach_database` 0 -> 31 of 43 lines, `translate_detach` 0 -> 30 of 50), alter.rs (+112, vtab rename), compound_select.rs (+156, UNION ALL across databases). Vtab files barely move (+4/+6): rename/destroy are a few lines and the DROP/rollback paths were already covered.

Regressions (per function, both pairs): btree.rs -92 / -138: `fill_cell_payload` overflow part (-62/-65 of 111 lines), `BTreeCursor::process_overflow_read` (-60/-61 of 79), `clear_overflow_pages` (-24 in the b pair); json/jsonb.rs -109 in the b pair only (`Jsonb::deserialize_string` -56). v3 writes and reads overflow pages less often; not explained (12% of sessions keep their rows in the MVCC store until the final checkpoint, but the loss is larger than that share). Data: runs/cov3/{v2,v3,v2b,v3b}.json, *-fn.txt, *-vs-*.txt.

## 2026-11-06 — round 3: triage

New probable Turso bugs (new folders in ../bugs/candidates/, repro + output + notes.md with facts and dedupe; repros in the new crate apidiff/repro3 on turso_core at 2709799da, `cargo run --release --bin NAME`; SQLite 3.50.4 side as turso-apidiff session replays with traces):
- `turso-vtab-registry-stale-after-rename-or-rollback` (vtab_rename_drop, vtab_drop_rollback): one connection; after `ALTER TABLE v RENAME TO w`, `DROP TABLE w` fails "Could not find Virtual Table to Destroy" and w stays; after `BEGIN; DROP TABLE v; ROLLBACK` (or SAVEPOINT/ROLLBACK TO), an INSERT into v panics "Could not find virtual table for VBegin" then "vtab must exist", and RENAME panics "Could not find virtual table for VRollback" (op_vrename). Same `syms.vtabs` lookup as round 2's vtab-other-connection candidate (by-name map not maintained on rename/rollback).
- `turso-checkpoint-restart-empty-write-stales-readers` (checkpoint_empty_write_stale): WAL, two connections; after a RESTART/TRUNCATE checkpoint (and FULL/PASSIVE when the writer starts after it), a write transaction that writes nothing (`BEGIN IMMEDIATE; COMMIT`, a no-op UPDATE or IF NOT EXISTS DDL, a vtab write) makes the other connection's open read transaction stale ("Database snapshot is stale"); SQLite: fine. `begin_write_tx` restarts the WAL header before anything is written.
- `turso-mvcc-commit-with-suspended-select-leaks-write-lock` (mvcc_commit_suspended_checkpoint): MVCC; COMMIT with a SELECT still open, then an autocommit write, then drop the SELECT: the write is visible only to its own connection, other connections get Busy for writes, checkpoint and journal_mode switch get Busy. Stepping the SELECT instead panics: open #9327/#9619 (also reached with no transaction control: an autocommit write finishing while the SELECT is suspended). Possibly comment material for #9619.
- `turso-mvcc-concurrent-ddl-after-autoincrement-insert-busy` (mvcc_concurrent_ddl): MVCC, one connection; `BEGIN CONCURRENT; INSERT INTO autoinc_table ...; ALTER/CREATE` -> Busy (no other transaction exists); without AUTOINCREMENT or without the INSERT the in-place upgrade (PR #9928) works.

Duplicates / known (no new folder): MVCC panic "transaction should exist while its cursor is active" = #9619/#9327 (open); MVCC index keeping the old key after REPLACE/UPDATE to a NOCASE-equal value ('B' -> 'b'; a covering index scan returns 'B' at once, SQLite's integrity_check on the checkpointed file reports "row 2 values differ from index t1_b") = #9285 (open; repro3/src/bin/mvcc_nocase_index_replace.rs kept for reference); MVCC plain-rowid gaps = #7474 (open); vtab written by a trigger rejected = deliberate check (translate/insert.rs:303), counted as a skip (not searched further); an attached database's snapshot kept after Busy on it in a transaction (later reads miss newer commits) = mechanism of round 2's turso-attached-busy-retry-stays-busy; SELECT-touching-only-temp/aux snapshot = #10825; suspended-writer data lost at finalize after a sibling finished (two connections, aux) = A.

Harness bugs fixed during round 3: one-shot statements stopped after 500 rows (bulk DELETE ... RETURNING compared as partial multisets); MVCC files unreadable by SQLite (now switched back to WAL); vtab dump used setup names (vtabs can be renamed/dropped now); attached-database dump assumed aux stays attached; `shrink` needs the session part of a findings file (extract from `== session` to `== trace`).

Not Turso (SQLite-side observations, not pursued):
- An `IF [NOT] EXISTS` DDL (e.g. `DROP TABLE IF EXISTS missing`) inside a transaction, or while another statement keeps the autocommit read transaction open, opens read transactions on every attached database (verify-all-schemas): afterwards `DETACH aux` fails "database aux is locked" and `PRAGMA aux.wal_checkpoint` returns SQLITE_LOCKED, although the transaction never used aux (r3-shake1 `detach`, `ckpt-slocked`; r3-smoke3 seed 7000603).
- SQLite returns SQLITE_BUSY to a connection that held a read transaction across another connection's RESTART/TRUNCATE checkpoint when it then writes (r3-shake1 `delbusy`), where nothing was committed meanwhile.
- An UPDATE that rewrites a row with identical bytes commits no WAL frame in SQLite (btree overwrite optimization), so other connections' snapshots stay current; Turso commits a frame/version and those snapshots become stale. Not a Turso bug; a source of two-connection noise (classified unspecified).

## Next round (suggested, round 4)
- Find the btree overflow regression: check which v2 sessions write overflow pages (probably the 700-char bulk rows combined with ALTER/INSERT ... SELECT doubling) and make sure v3 keeps them (MVCC sessions do not write the btree until the final checkpoint; maybe add a mid-session TRUNCATE checkpoint in MVCC sessions, which would also exercise checkpoint_state_machine more).
- Lower the weights of the patterns that now end sessions as known (vtab DROP+ROLLBACK -> 4.3k panics, RESTART/TRUNCATE checkpoints on two connections) until the candidates are fixed; this returns ~10% of sessions to clean comparisons.
- MVCC: two-connection BEGIN CONCURRENT sessions need a different oracle (commit-time conflicts): a serializability check of committed concurrent transactions (replay the committed ones serially in commit order in SQLite and compare), rather than lockstep comparison.
- ATTACH: vtabs and TEMP tables on the second connection (temp is per connection: separate w-tables per connection), ATTACH of the same file under two names, and attached databases in MVCC sessions once Turso supports a WAL attachment of an MVCC main (attach.rs checks the mode).
- Vtab coverage is saturated with the appvt module; next gains need xBestIndex constraints (filtered scans with argv) and the savepoint hooks once Turso's trait has them.

## 2026-11-06 — reproductions moved to the bug folders

The repro crates are gone from apidiff: each filed bug folder in ../bugs now holds `repro.rs` (posted.md's code block byte for byte; for #11538, #11539, #11543, #11544, #11546 the old `issue_*.rs` bins differed from posted.md only in comments), `repro_sqlite.rs` (the former `issue_*_sqlite.rs` bins) or `repro.sql` / `repro-sqlite.sql`, fresh `out-turso.txt` / `out-sqlite.txt` (2709799da / 3.50.4), built by ../bugs/Cargo.toml. vtab-ext moved to ../bugs/memvt (the harness never used it; its scripts became the folders' `repro-sqlite.sql`). repro3's bins and lib went to ../bugs/candidates (Cargo.toml, repro3-lib.rs), round 1's bug-A bins to candidates/turso-sibling-statement-end-stops-suspended-select. Dropped: the triage bins of repro and repro2 (variants and the turso_core versions of the filed repros), and repro3's mvcc_nocase_index_replace (#9285 is open and filed by others; it is in git history). References above to apidiff/repro, repro2, repro3 and vtab-ext point to that history.

## 2026-11-06: generator files
The generators moved to src/gen/: mod.rs (the shared action language, value pools, version choice and session scripts), v1.rs (round 1, formerly gen.rs, also the building blocks v2 and v3 reuse), v2.rs (formerly gen2.rs), v3.rs (formerly gen3.rs). No behaviour change: `one SEED --gen N` gives identical output before and after for N = 1, 2, 3 (6 seeds each). The file names in the round sections above are the old ones.
