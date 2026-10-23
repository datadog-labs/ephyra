**Title:** "database disk image is malformed" or wrong answer: row-value IN (SELECT ...) over an added column of a WITHOUT ROWID table

```sql
CREATE TABLE t1(a PRIMARY KEY) WITHOUT ROWID;
INSERT INTO t1 VALUES(1.5), (2.5);
ALTER TABLE t1 ADD COLUMN b DEFAULT 5;
PRAGMA integrity_check;
SELECT (1.5, 5) IN (SELECT a, b FROM t1);
SELECT (1.5, 6) NOT IN (SELECT a, b FROM t1);
```

Output from trunk (check-in 5bd093e261a7, 2026-10-16 18:11), built from the source tarball with default options. 3.54.0 gives the same:

```
ok
Error near line 5: database disk image is malformed
Error near line 6: database disk image is malformed
```

Expected:

```
ok
1
1
```

The database is fine: integrity_check says ok, and the same queries are right if `b` is declared in CREATE TABLE, if the table has a rowid, or if the rows are inserted after the ALTER. With an integer key there is no error, only a wrong answer: with `INSERT INTO t3 VALUES(1), (2), (3)` before the same ALTER, `SELECT (2, 5) IN (SELECT a, b FROM t3)` gives NULL. When the IN drives a loop the rows go missing: with `CREATE TABLE t2(x, y); INSERT INTO t2 VALUES(1.5, 5), (2.5, 6); CREATE INDEX t2xy ON t2(x, y);`, `SELECT * FROM t2 WHERE (x, y) IN (SELECT a, b FROM t1)` returns no rows instead of `1.5|5`. A composite key fails the same way (`PRIMARY KEY(x, y)`, then `ADD COLUMN z`, then `(1.5, 2, 7) IN (SELECT x, y, z FROM t4)`).

**Cause, as far as we can tell.** EXPLAIN QUERY PLAN shows `USING INDEX sqlite_autoindex_t1_1 FOR IN-OPERATOR`, and the membership test probes the PRIMARY KEY b-tree with a two-field key, (a, b):

```
OpenRead       1     2     0     k(1,)          0   root=2 iDb=0; sqlite_autoindex_t1_1
Real           0     3     1073217536                0   r[3]=1.5
Integer        5     4     0                    0   r[4]=5
Affinity       3     2     0     AA             0   affinity(r[3..4])
Found          1     17    3     2              0   key=r[3..4]
```

`sqlite3FindInIndex()` accepts any index with `nColumn >= nExpr` whose first nExpr columns are the RHS columns. For the PRIMARY KEY index of a WITHOUT ROWID table, nColumn counts every table column, not just the nKeyCol key columns, so `b` qualifies. Rows written before the ADD COLUMN have no field for `b`. When the first field compares equal, `sqlite3VdbeRecordCompareWithSkip()` reports the short record as corrupt ("Corrupt index", vdbeaux.c), which `sqlite3BtreeIndexMoveto()` turns into SQLITE_CORRUPT for the Found. With an integer key, `vdbeRecordCompareInt()` calls the comparator with bSkip, which reads the byte after the record header as the next serial type, so the probe fails without an error and the NULL-scan fallback reads `b` from the index cursor as NULL, giving NULL. In the loop case the same OP_Column on the index cursor gives NULL instead of the default, so the row is skipped. This is the same thing as the NULLS LAST case fixed by check-in a2a20c54a2 (bug forum eee345f931), reached through the IN operator instead. `whereLoopAddBtreeIndex()`, `whereKeyStats()` and, since that fix, `wherePathSatisfiesOrderBy()` already stop at nKeyCol on such an index; doing the same here fixes every query above:

```diff
--- a/src/expr.c
+++ b/src/expr.c
@@ -3399,6 +3399,14 @@
           testcase( pIdx->nColumn==BMS-2 );
           testcase( pIdx->nColumn==BMS-1 );
           if( pIdx->nColumn>=BMS-1 ) continue;
+          if( nExpr>pIdx->nKeyCol
+           && IsPrimaryKeyIndex(pIdx) && !HasRowid(pTab)
+          ){
+            /* The PK of a WITHOUT ROWID table holds all table columns, but
+            ** fields past nKeyCol may be missing from records written before
+            ** ALTER TABLE ADD COLUMN. Do not look them up in the b-tree. */
+            continue;
+          }
           if( mustBeUnique ){
             if( pIdx->nKeyCol>nExpr
              ||(pIdx->nColumn>nExpr && !IsUniqueIndex(pIdx))
```

With it, the rowvalue*, without_rowid*, alter*, where*, join* and in* tests pass as before, and veryquick shows no new failures. Take it as a pointer to the cause rather than a proposed fix.

**Versions** (official builds, default options): 3.40.0, 3.44.0, 3.50.4, 3.51.0, 3.53.4, 3.54.0 and trunk give the error for both queries above. 3.20.0 through 3.39.0 give no error but wrong answers: `1` for the first query (right) and `0` for the second (expected `1`), NULL for the integer-key query and no rows for the loop query. The error appears in 3.40.0, as with the NULLS LAST case.

We found it with a property-based test generator for SQLite databases and queries: a three-way self-join of a WITHOUT ROWID table with an added column, one of whose ON clauses was `(a0.c0, a2.c0) IN (SELECT c1, c0 FROM t0 AS b0)`, failed with "database disk image is malformed".
