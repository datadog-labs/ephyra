**Title:** "database disk image is malformed" or missing rows: ORDER BY an added column NULLS LAST on a WITHOUT ROWID table

```sql
CREATE TABLE t1(a PRIMARY KEY) WITHOUT ROWID;
INSERT INTO t1 VALUES(1.5);
ALTER TABLE t1 ADD COLUMN b;
PRAGMA integrity_check;
SELECT * FROM t1 WHERE a IN (1.5, 2) ORDER BY b NULLS LAST;

CREATE TABLE t2(a PRIMARY KEY) WITHOUT ROWID;
INSERT INTO t2 VALUES(1), (2);
ALTER TABLE t2 ADD COLUMN b DEFAULT 5;
INSERT INTO t2 VALUES(3, NULL);
SELECT * FROM t2 WHERE a IN (1, 2, 3) ORDER BY b NULLS LAST;
```

Output from trunk (check-in bbdd13370de9, 2026-10-14 20:07), built from the source tarball with default options. 3.54.0 gives the same:

```
ok
Error near line 5: database disk image is malformed
2|5
3|
```

Expected:

```
ok
1.5|
1|5
2|5
3|
```

The database is fine: integrity_check says ok, and the same queries are right if `b` is declared in CREATE TABLE, if the table has a rowid, or if the ORDER BY has no NULLS LAST. With an integer key there is no error, only wrong rows: `INSERT INTO t1 VALUES(1)` and `... WHERE a IN (1, 2) ORDER BY b NULLS LAST` returns nothing. `ORDER BY b DESC NULLS FIRST` and a composite key (`PRIMARY KEY(a,b)`, `WHERE a=1 AND b IN (1,2) ORDER BY c NULLS LAST` with c added) fail the same way.

**Cause, as far as we can tell.** EXPLAIN shows the big-null loop seeking on the table's PRIMARY KEY b-tree with a two-field key, (a, NULL):

```
Null           0     3     0                r[3]=NULL
Integer        1     1     0                r[1]=1; NULL-scan pass ctr
SeekGT         2     31    2     2          key=r[2..3]
```

In `wherePathSatisfiesOrderBy()`, the loop over index columns runs up to `pIndex->nColumn`, which for a WITHOUT ROWID PRIMARY KEY index includes every table column, not just the key. The ASC NULLS LAST term on `b` matches column `j==nEq`, so WHERE_BIGNULL_SORT is set even though `b` is not a key column. The seek key then includes `b`. Rows written before the ADD COLUMN have no field for `b`, and `sqlite3VdbeRecordCompareWithSkip()` reports a record shorter than the key as corrupt ("Corrupt index"). On the integer fast path (`vdbeRecordCompareInt()` with bSkip) it reads the byte after the record header as the next serial type, so rows go missing. `whereLoopAddBtreeIndex()` already never constrains PRIMARY KEY columns past nKeyCol; doing the same here fixes every query above:

```diff
--- a/src/where.c
+++ b/src/where.c
@@ -5457,7 +5457,14 @@
           }
         }
         if( isMatch && (pOrderBy->a[i].fg.sortFlags & KEYINFO_ORDER_BIGNULL) ){
-          if( j==pLoop->u.btree.nEq ){
+          if( j==pLoop->u.btree.nEq
+           && (j<nKeyCol || pIndex==0
+               || pIndex->idxType!=SQLITE_IDXTYPE_PRIMARYKEY)
+          ){
             pLoop->wsFlags |= WHERE_BIGNULL_SORT;
           }else{
             isMatch = 0;
```

With it, the alter*, corrupt*, descidx*, in*, index*, nulls*, orderby*, select*, where* and without_rowid* tests we ran pass. Take it as a pointer to the cause rather than a proposed fix.

**Versions** (official amalgamations or tarballs, default options): 3.30.0 and 3.39.0 give no error but wrong rows: the second query returns `1|5 1|5 2|5 2|5 3|`, and in 3.39.0 `ORDER BY b DESC NULLS FIRST` returns only `3|`. 3.40.0 gives "malformed" for the first query and the right rows for the second. 3.50.4, 3.53.4, 3.54.0 and trunk give both wrong results above. Releases before 3.30.0 don't have NULLS LAST.

We found it with a property-based test generator for SQLite databases and queries: adding INDEXED BY to a self-join over such a table turned an empty result into "database disk image is malformed".
