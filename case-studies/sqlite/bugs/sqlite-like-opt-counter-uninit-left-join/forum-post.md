# Reply (bug forum thread 74bf8db543)

Posted to the SQLite bug forum as https://sqlite.org/bugs/forumpost/70e3174f54 on 2026-10-27, as a reply in thread https://sqlite.org/bugs/forumpost/74bf8db543. The text below is the post as written.

---

Hi Dan,

Possible regression:

Since check-in 08e979525ff1 the LIKE loop counter is set after the == terms are coded, and an == term that is NULL now jumps past the point where the counter is set. In a LEFT JOIN, the NULL row then reaches the OP_DecrJumpZero with the counter never set:

```sql
CREATE TABLE t1(a, b);
CREATE INDEX t1ab ON t1(a, b);
CREATE TABLE t2(x);
INSERT INTO t2 VALUES(1);
SELECT 1 FROM t2 LEFT JOIN t1 ON t1.b GLOB 'a*' AND t1.a = NULL;
```

Trunk 46d3df136172 (2026-10-26) built with `--enable-debug`:

```
Assertion failed: (memIsValid(&aMem[pOp->p1])), function sqlite3VdbeExec, file sqlite3.c, line 97499.
```
