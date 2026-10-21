**Title:** An index on a VIRTUAL generated column with no type returns an integral REAL as an INTEGER

```sql
CREATE TABLE t(a REAL, b AS (a) VIRTUAL);
INSERT INTO t(a) VALUES(1.0);
CREATE INDEX i ON t(b);
SELECT b, typeof(b) FROM t NOT INDEXED;
SELECT b, typeof(b) FROM t INDEXED BY i;
SELECT b, typeof(b) FROM t WHERE b = 1;
```

Output from trunk (check-in dabff5c1cde6, 2026-10-14 16:11), built from the source tarball with default options. 3.54.0, 3.53.4 and 3.50.4 give the same:

```
1.0|real
1|integer
1|integer
```

Expected `1.0|real` three times. b has no declared type, so it has no affinity and its value is a's value, the REAL 1.0. That is what NOT INDEXED returns. Whenever b is read from the index instead, it comes back as the integer 1. The difference shows in `typeof(b)`, `quote(b)`, `b || ''` and `b / 2` (0 instead of 0.5). It also changes which rows `WHERE typeof(b) = 'real'` keeps (0 rows with the index, 1 without), and the results of ORDER BY b, GROUP BY b, DISTINCT b, sum(b), min/max through a covering index, and partial indexes on b. `b BLOB AS (a)` behaves the same. STORED columns, declared types REAL/INT/NUMERIC/TEXT, and expression indexes such as `t((a))` or `t(a+0)` give the right answer.

Only entries that CREATE INDEX or REINDEX build from rows already in the table are wrong. Rows inserted while the index exists are stored as 1.0. VACUUM keeps the wrong entries. PRAGMA integrity_check reports ok, since 1 = 1.0.

**Cause, as far as we can tell.** `sqlite3GenerateIndexKey()` in delete.c calls `sqlite3ExprCodeLoadIndexColumn()` for each index column. When `aiColumn[j]>=0`, it then deletes a trailing OP_RealAffinity, so that an integral REAL is stored in the index in its compact integer form. That is fine for a REAL column, which gets OP_RealAffinity again when it is read back. For a VIRTUAL column, the trailing OP_RealAffinity is the one `sqlite3ExprCodeGeneratedColumn()` coded for the REAL column `a` in the generation expression. With no affinity of its own, b adds no OP_Affinity after it. EXPLAIN CREATE INDEX shows `Column 1 0 10` followed by `Noop`. b is not REAL, so nothing converts the integer back when b is read from the index. Leaving the opcode in place for VIRTUAL columns fixes all the cases above for us:

```diff
--- a/src/delete.c
+++ b/src/delete.c
@@ -1005,7 +1005,9 @@
       continue;
     }
     sqlite3ExprCodeLoadIndexColumn(pParse, pIdx, iDataCur, j, regBase+j);
-    if( pIdx->aiColumn[j]>=0 ){
+    if( pIdx->aiColumn[j]>=0
+     && (pIdx->pTable->aCol[pIdx->aiColumn[j]].colFlags & COLFLAG_VIRTUAL)==0
+    ){
       /* If the column affinity is REAL but the number is an integer, then it
       ** might be stored in the table as an integer (using a compact
       ** representation) then converted to REAL by an OP_RealAffinity opcode.
```

With the patch, the forms that were right before give the same output, and the gencol1, index*, indexexpr*, reindex, vacuum, intreal, affinity*, types*, without_rowid*, update, delete, insert*, minmax*, distinct* and sort* tests we ran pass. Take it as a pointer to the cause rather than a proposed fix.

**Versions** (official amalgamations, default options): 3.31.0, 3.31.1, 3.32.0, 3.35.0, 3.37.0, 3.38.0 and 3.39.0 return the integer (for example with `WHERE b > 0` or `ORDER BY b`). 3.40.0 and 3.40.1 return 1.0, because they recompute b from the table, although the index already holds the integer. 3.41.0, 3.42.0, 3.45.0, 3.50.4, 3.53.4, 3.54.0 and trunk dabff5c1cde6 return the integer.

We found it with a property-based test generator for SQLite databases and queries: in the union of `WHERE p`, `WHERE NOT p` and `WHERE p IS NULL`, the same row's b came back as REAL in one part and as INTEGER in another.
