**Title:** "database disk image is malformed" or missing rows: row-value range over an added column of a WITHOUT ROWID table

```sql
CREATE TABLE t1(a PRIMARY KEY) WITHOUT ROWID;
INSERT INTO t1 VALUES(1.5), (2.5);
ALTER TABLE t1 ADD COLUMN b DEFAULT 5;
PRAGMA integrity_check;
SELECT * FROM t1 WHERE (a, b) > (1.5, 0);
SELECT * FROM t1 WHERE (a, b) < (2.5, 9);
```

Output from trunk (check-in 668541c592b8, 2026-10-16 14:31), built from the source tarball with default options. 3.54.0 gives the same:

```
ok
Error near line 5: database disk image is malformed
1.5|5
```

Expected:

```
ok
1.5|5
2.5|5
1.5|5
2.5|5
```

Both rows have b = 5, so both satisfy both comparisons. The database is fine: integrity_check says ok, and the same queries are right if `b` is declared in CREATE TABLE, if the table has a rowid, or if the rows are inserted after the ALTER. With an integer key there is no error, only missing rows: with `INSERT INTO t1 VALUES(1), (2), (3)` before the ALTER, `WHERE (a, b) > (1, 0) AND (a, b) < (3, 9)` returns only `2|5`. A composite key fails the same way (`PRIMARY KEY(x, y)`, then `ADD COLUMN z`, then `WHERE (x, y, z) > (1, 1, NULL)`), and so does a RIGHT JOIN whose ON clause holds the comparison (`SELECT * FROM t2 RIGHT JOIN t1 ON (t1.a, t1.b) > (1.5, 0)`).

**Cause, as far as we can tell.** EXPLAIN QUERY PLAN shows `SEARCH t1 USING PRIMARY KEY ((a,b)>(?,?))`, and the seek on the PRIMARY KEY b-tree uses a two-field key, (a, b):

```
Real           0     1     1073217536       r[1]=1.5
Integer        0     2     0                r[2]=0
SeekGE         1     20    1     2          key=r[1..2]
```

`whereRangeVectorLen()` caps the vector at `pIdx->nColumn - nEq`. For the PRIMARY KEY index of a WITHOUT ROWID table, nColumn counts every table column, not just the nKeyCol key columns, so `b` is taken into the seek key. Rows written before the ADD COLUMN have no field for `b`. `sqlite3VdbeRecordCompareWithSkip()` reports a record shorter than the key as corrupt ("Corrupt index", vdbeaux.c), which `sqlite3BtreeIndexMoveto()` turns into SQLITE_CORRUPT for the SeekGE. The upper bound is checked with IdxGT, which ignores errCode: the compare returns 0 and the loop stops at the 2.5 row. With an integer key, `vdbeRecordCompareInt()` calls the comparator with bSkip, which reads the byte after the record header as the next serial type, so rows go missing without an error. This is the same thing as the NULLS LAST case fixed by check-in a2a20c54a2 (bug forum eee345f931), reached through a row-value range instead. `whereLoopAddBtreeIndex()` and, since that fix, `wherePathSatisfiesOrderBy()` already stop at nKeyCol on such an index; doing the same here fixes every query above:

```diff
--- a/src/where.c
+++ b/src/where.c
@@ -3157,9 +3157,16 @@
   WhereTerm *pTerm     /* The vector inequality constraint */
 ){
   int nCmp = sqlite3ExprVectorSize(pTerm->pExpr->pLeft);
+  int nCol = pIdx->nColumn;
   int i;
 
-  nCmp = MIN(nCmp, (pIdx->nColumn - nEq));
+  /* If this is the PK of a WITHOUT ROWID table, do not use any field
+  ** past nKeyCol. If ALTER TABLE has been used to add columns to the
+  ** table, those fields may not be present in all records.  */
+  if( IsPrimaryKeyIndex(pIdx) && !HasRowid(pIdx->pTable) ){
+    nCol = pIdx->nKeyCol;
+  }
+  nCmp = MIN(nCmp, (nCol - nEq));
   for(i=1; i<nCmp; i++){
     /* Test if comparison i of pTerm is compatible with column (i+nEq)
     ** of the index. If not, exit the loop.  */
```

With it, the join*, window*, rowvalue*, alter*, without_rowid*, where* and nulls* tests pass as before, and veryquick shows no new failures. Take it as a pointer to the cause rather than a proposed fix.

**Versions** (official builds, default options): 3.40.0, 3.44.0, 3.50.4, 3.51.0, 3.53.4, 3.54.0 and trunk give the error for the first query and the missing row for the second (for the integer-key query, 3.40.0 and 3.44.0 return `1|5 2|5`, the later ones `2|5`). 3.20.0 through 3.39.0 give the right rows for every query above, although the plan is the same; the error and the missing rows appear in 3.40.0, as with the NULLS LAST case.

We found it with a property-based test generator for SQLite databases and queries: a query with `t2 AS a0 LEFT JOIN t0 AS a1 ON (a0.c0, a0.c1, a0.c2) > (...)` returned rows, and the same query written as `t0 AS a1 RIGHT JOIN t2 AS a0 ON ...` failed with "database disk image is malformed".
