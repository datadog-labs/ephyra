**Title:** A statement that gets SQLITE_BUSY from an attached database keeps its statement transaction open

```c
#include <stdio.h>
#include "sqlite3.h"

static void finish(const char *zName, sqlite3_stmt *p){
  while( sqlite3_step(p)==SQLITE_ROW ){}
  printf("%s: %s\n", zName, sqlite3_errstr(sqlite3_finalize(p)));
}

int main(void){
  sqlite3 *db, *db2;
  sqlite3_stmt *pP, *pA, *pB, *pS;
  remove("m.db"); remove("x.db");
  sqlite3_open("m.db", &db);
  sqlite3_exec(db, "ATTACH 'x.db' AS x;"
                   "CREATE TABLE t(a INTEGER PRIMARY KEY, b UNIQUE);"
                   "INSERT INTO t VALUES(1,1),(2,2),(3,3);"
                   "CREATE TABLE x.u(v);"
                   "CREATE TEMP TRIGGER tr AFTER UPDATE ON t BEGIN"
                   "  INSERT INTO u VALUES(new.b);"
                   "END;", 0, 0, 0);
  sqlite3_open("x.db", &db2);
  sqlite3_exec(db2, "BEGIN IMMEDIATE", 0, 0, 0);   /* x.db is locked */

  /* Autocommit mode.  Each UPDATE also writes x.db through the trigger. */
  sqlite3_prepare_v2(db, "UPDATE t SET b=b WHERE a=99", -1, &pP, 0);
  sqlite3_prepare_v2(db, "UPDATE t SET b=b+10", -1, &pA, 0);
  sqlite3_prepare_v2(db,
      "UPDATE t SET b=CASE a WHEN 2 THEN 13 ELSE b+100 END", -1, &pB, 0);
  printf("P: %s\n", sqlite3_errstr(sqlite3_step(pP)));
  printf("A: %s\n", sqlite3_errstr(sqlite3_step(pA)));
  printf("B: %s\n", sqlite3_errstr(sqlite3_step(pB)));
  sqlite3_exec(db2, "COMMIT", 0, 0, 0);            /* x.db is unlocked */
  finish("A", pA);                                 /* retry, oldest first */
  finish("B", pB);
  finish("P", pP);

  sqlite3_prepare_v2(db, "SELECT (SELECT group_concat(a||'|'||b, ' ') FROM t),"
                         " (SELECT group_concat(v) FROM x.u)", -1, &pS, 0);
  sqlite3_step(pS);
  printf("t: %s\nx.u: %s\n", sqlite3_column_text(pS, 0),
         sqlite3_column_text(pS, 1));
  sqlite3_finalize(pS);
  sqlite3_close(db2);
  sqlite3_close(db);
  return 0;
}
```

Output from trunk (check-in 6b81ddfd0690, 2026-11-03 15:24). 3.54.1 and 3.34.1 give the same:

```
P: database is locked
A: database is locked
B: database is locked
A: not an error
B: constraint failed
P: not an error
t: 1|111 2|12 3|13
x.u: 11,12,13
```

With `-DSQLITE_DEBUG`, all three fail when A is finished (3.54.1 at line 90712, 3.34.1 at line 80924):

```
P: database is locked
A: database is locked
B: database is locked
Assertion failed: (p->iStatement==(db->nStatement+db->nSavepoint)), function vdbeCloseStatement, file sqlite3.c, line 90926.
```

Expected: t ends with b = 11, 12 and 13. The `sqlite3_step()` documentation says a statement that gets SQLITE_BUSY outside an explicit transaction can be retried, and B's UNIQUE constraint failure should undo all of B. Instead B's change to row 1 of t (b = 111) is committed, while the row its trigger inserted into x.u (111) is rolled back. `PRAGMA integrity_check` returns ok. 3.8.0 gives the same rows in a different order, and 3.7.0 the same result once the multi-row INSERT is split into single-row INSERTs. It takes a statement that writes both main and an attached database (here through the TEMP trigger) and another statement that runs before it is retried.

**Cause, as far as we can tell.** Each UPDATE opens its statement transaction when its `OP_Transaction` on main succeeds. When the `OP_Transaction` on x.db then gets SQLITE_BUSY, it sets `p->pc` so that the opcode is retried and returns, leaving `p->iStatement` and `db->nStatement` unchanged, so the statement transaction stays open while the application runs other statements. Here B holds statement savepoint 1 on main, nested in A's savepoint 0. When A finishes first, releasing savepoint 0 also discards savepoint 1 on main. On its retry, B opens savepoint 1 on x.db only, so its rollback undoes its x.db changes but not its main changes.

The patch below handles SQLITE_BUSY from any `OP_Transaction` after the first one of a statement. It releases the statement transaction, if one is open, and makes the next step restart at the first `OP_Transaction`. Nothing has been written at that point, and on a database that already has the transaction it needs `OP_Transaction` only checks the schema cookie again, so the restart opens the statement transaction again, correctly nested. With it, both builds give the expected output above:

```diff
--- a/src/vdbe.c
+++ b/src/vdbe.c
@@ -4288,6 +4288,24 @@
       if( (rc&0xff)==SQLITE_BUSY ){
         p->pc = (int)(pOp - aOp);
         p->rc = rc;
+        if( aOp[0].opcode==OP_Init && aOp[0].p2<p->pc ){
+          /* Earlier OP_Transaction ops of this statement have succeeded.
+          ** Their transactions might be rolled back before this statement
+          ** is retried, and any statement transaction they opened must not
+          ** be held while control returns to the application, which might
+          ** run other statements first.  Nothing has been written yet, so
+          ** release the statement transaction and restart from the first
+          ** OP_Transaction (at aOp[0].p2) on the next step. */
+          if( p->iStatement ){
+            int rc2 = sqlite3VdbeCloseStatement(p, SAVEPOINT_RELEASE);
+            if( rc2 ){
+              rc = rc2;
+              goto abort_due_to_error;
+            }
+          }
+          p->pc = aOp[0].p2;
+        }
+        assert( p->iStatement==0 );
         goto vdbe_return;
       }
       goto abort_due_to_error;
```

The patch restarts the statement even when it holds no statement transaction. A statement without one (for example the same UPDATE on a t without the UNIQUE constraint, inside BEGIN) otherwise resumes after the `OP_Transaction` on main. If the application runs ROLLBACK after the SQLITE_BUSY, main is left with a read transaction (the pending statement still counts as a reader), and when the statement is retried it writes to main without a write transaction. A debug build then fails as below; a release build shows the update to that connection, while a new connection still sees the old values:

```
Assertion failed: (eLock==READ_LOCK || (p==pBt->pWriter && p->inTrans==TRANS_WRITE)), function querySharedCacheTableLock, file sqlite3.c, line 73228.
```

The assertion in the first case is the same as in [64c52890a4](https://sqlite.org/bugs/forumpost/64c52890a4), but it is reached through the SQLITE_BUSY return in `OP_Transaction` rather than `OP_ResultRow`, and the patch there does not change this case.

**Versions**: the same output in 3.7.0 (with single-row INSERTs), 3.8.0 (rows in a different order), 3.34.1, 3.54.1 and trunk.

We found it with a property-based test generator for SQLite that builds VDBE programs directly.
