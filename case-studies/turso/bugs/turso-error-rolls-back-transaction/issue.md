DRAFT, not filed. Duplicate of open issue #6864; if anything, post as a comment there (it widens the scope).

Title: A runtime error in any statement rolls back the whole explicit transaction (not only JSON errors)

#6864 shows this with a malformed-JSON SELECT. The same happens for other runtime errors (integer overflow here), for INSERT as well as SELECT, and under SAVEPOINT as well as BEGIN. Uncommitted DDL is lost too, so the next statement fails with "no such table".

Reproduction (`tursodb -q -m list x.db < repro.sql`):

```sql
CREATE TABLE t0(c0);
BEGIN;            -- SAVEPOINT s0; behaves the same
INSERT INTO t0 VALUES (1);
CREATE TABLE t5(c0 INTEGER, c1 TEXT) STRICT;
SELECT abs(-9223372036854775808);   -- runtime error: integer overflow
SELECT count(*) FROM t0;
SELECT * FROM t5;
COMMIT;
SELECT count(*) FROM t0;
```

Turso (7641fad2):
```
Runtime error: integer overflow
0
Parse error: no such table: t5
Runtime error: Transaction error: cannot commit - no transaction is active
0
```

SQLite 3.50.4 (trunk f69fd5f5 gives the same):
```
Runtime error near line 5: integer overflow
1
1
```

Expected: the failing statement is aborted and the transaction stays open. SQLite rolls back the whole transaction only for a few error codes (SQLITE_FULL, IOERR, NOMEM, BUSY) or for ON CONFLICT ROLLBACK. `INSERT INTO t0 VALUES(abs(-9223372036854775808))` in place of the SELECT gives the same rollback.

Likely cause: in `core/vdbe/mod.rs`, `Program::abort`, the fallback `_ =>` arm with `TxnCleanup::None` calls `rollback_current_txn` when `!self.connection.get_auto_commit() && err.is_some()`. So any error in a statement that did not open a statement savepoint (reads, single-row inserts) ends the user's transaction.

Tested on Turso main 7641fad2 (2026-10-13).
