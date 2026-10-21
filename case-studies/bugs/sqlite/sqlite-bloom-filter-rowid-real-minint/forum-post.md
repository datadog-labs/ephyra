**Title:** A join on an INTEGER PRIMARY KEY misses the REAL key -9223372036854775808.0 when the plan has a Bloom filter

```sql
CREATE TABLE t1(a);
CREATE TABLE t2(b INTEGER PRIMARY KEY);
INSERT INTO t1 VALUES(-9223372036854775808.0);
INSERT INTO t2 VALUES(-9223372036854775808);
ANALYZE sqlite_schema;
INSERT INTO sqlite_stat1 VALUES('t1',NULL,'100'),('t2',NULL,'10');
ANALYZE sqlite_schema;
SELECT count(*) FROM t1, t2 WHERE b = a;
SELECT count(*) FROM t1, t2 WHERE b = a AND b <> 5;
```

Output from trunk (check-in dc388e83b05d, 2026-10-10 19:19), built from the source tarball with default options:

```
1
0
```

Expected `1` both times. -9223372036854775808.0 is exactly -2^63, so `b = a` is true for the one pair (`SELECT a = -9223372036854775808 FROM t1` gives 1), and `b <> 5` is true as well. The first query finds the row since check-in 987ba5a3dc5f2a22 (in 3.54.0). That check-in fixed `rowid = -9223372036854775808.0` for user forum post 989b0d4471 ("Unexpected execution result of two result-equivalent queries", 2026-07-19), and its follow-up 4a66e328a0 fixed the +2^63 side (bug forum post f5d54eb0c5, 2026-09-23).

The second query's plan adds `BLOOM FILTER ON t2 (b=?)` before `SEARCH t2 USING INTEGER PRIMARY KEY (rowid=?)`. The stat1 rows and `b <> 5` are only there to get that plan. With `.testctrl optimizations -BloomFilter` it gives 1, and so does `WHERE b+0 = a AND b <> 5`. It still gives 0 with `a = b`, with `t2.rowid = a`, with `a` declared REAL, with the text '-9223372036854775808.0' in `a`, and as `EXISTS (SELECT 1 FROM t2 WHERE b = a AND b <> 5)`. A Bloom filter in front of an ordinary index on `b INTEGER`, or in front of a WITHOUT ROWID primary key, finds the row.

A foreign key check against an INTEGER PRIMARY KEY parent also rejects this value:

```sql
PRAGMA foreign_keys = ON;
CREATE TABLE p(id INTEGER PRIMARY KEY);
CREATE TABLE c(r REFERENCES p);
INSERT INTO p VALUES(-9223372036854775808);
INSERT INTO c VALUES(-9223372036854775808.0);  -- FOREIGN KEY constraint failed
```

The parent key equals the child key. With `p(id INTEGER UNIQUE)` and `REFERENCES p(id)`, the same insert succeeds.

**Cause, as far as we can tell.** When the INTEGER PRIMARY KEY loop has a Bloom filter, `sqlite3WhereCodeOneLoopStart()` (case 2) codes `OP_MustBeInt` before `OP_Filter` and `OP_SeekRowid`, and `filterPullDown()` does the same:

```c
    if( pLevel->regFilter ){
      sqlite3VdbeAddOp2(v, OP_MustBeInt, iRowidReg, addrMiss);
      VdbeCoverage(v);
      sqlite3VdbeAddOp4Int(v, OP_Filter, pLevel->regFilter, addrMiss,
                           iRowidReg, 1);
      VdbeCoverage(v);
      filterPullDown(pParse, pWInfo, iLevel, addrNxt, notReady);
    }
    sqlite3VdbeAddOp3(v, OP_SeekRowid, iCur, addrMiss, iRowidReg);
```

Check-in 8e14c351b29bb434 (3.40.0, forum post f61a8b7053) added that `OP_MustBeInt` for TEXT keys such as '14'. It applies NUMERIC affinity, and `sqlite3VdbeIntegerAffinity()` leaves -9223372036854775808.0 as a REAL (`ix>SMALLEST_INT64`, ticket #3922). So `OP_MustBeInt` skips the row before it reaches `OP_SeekRowid`, which has accepted that value since 987ba5a3dc. For an INTEGER PRIMARY KEY parent, `fkLookupParent()` also codes `OP_MustBeInt` before `OP_NotExists`, so the foreign key check would fail in the same way.

Letting `OP_MustBeInt` accept exactly that value gives the expected answer for every query above:

```diff
--- src/vdbe.c
+++ src/vdbe.c
@@ -2168,12 +2168,16 @@
   if( (pIn1->flags & MEM_Int)==0 ){
     applyAffinity(pIn1, SQLITE_AFF_NUMERIC, encoding);
     if( (pIn1->flags & MEM_Int)==0 ){
-      VdbeBranchTaken(1, 2);
-      if( pOp->p2==0 ){
-        rc = SQLITE_MISMATCH;
-        goto abort_due_to_error;
+      if( (pIn1->flags & MEM_Real)!=0 && pIn1->u.r==-9223372036854775808.0 ){
+        pIn1->u.i = SMALLEST_INT64;  /* exact, as in OP_SeekRowid */
       }else{
-        goto jump_to_p2;
+        VdbeBranchTaken(1, 2);
+        if( pOp->p2==0 ){
+          rc = SQLITE_MISMATCH;
+          goto abort_due_to_error;
+        }else{
+          goto jump_to_p2;
+        }
       }
     }
   }
```

It also changes the other users of `OP_MustBeInt`. An INTEGER PRIMARY KEY then accepts -9223372036854775808.0, as it already accepts -5.0. It also accepts the text '-9223372036854775809' (which `WHERE b = '-9223372036854775809'` already matches). Both are stored as -9223372036854775808, where trunk fails with "datatype mismatch". `LIMIT -9223372036854775808.0` then means no limit. The veryquick tests, tkt3922.test and func4.test give the same results here with and without the diff. Dropping `ix>SMALLEST_INT64` from `sqlite3VdbeIntegerAffinity()` would also fix these queries, but it breaks tkt3922.2, tkt3922.3 and func4-3.18, so that exclusion looks deliberate. We have only looked at these cases, so take the diff as a pointer to the cause rather than a proposed fix.

Partial indexes with `WHERE b = -9223372036854775808.0` ([forum post 35992eec46](https://sqlite.org/bugs/forumpost/35992eec46)) and constant propagation also go wrong on this value, through the same exclusion in `sqlite3VdbeIntegerAffinity()`. For example, with b an INT column holding the integer -9223372036854775808, `SELECT count(*) FROM t WHERE b = -9223372036854775808.0 AND typeof(b) = 'integer'` gives 0, and 1 with `.testctrl optimizations -PropagateConst`.

**Versions** (official amalgamations or tarballs, default options):
- 3.7.17, 3.20.0, 3.28.0, 3.37.0, 3.38.0, 3.39.0, 3.40.0, 3.45.0, 3.51.0 and 3.53.4 give `0` and `0`, and so do the other releases we tried between them. The first `0` was the bug that 987ba5a3dc fixed.
- 3.54.0 and trunk dc388e83b05d give `1` and `0`.
- The Bloom filter is in the plan from 3.38.0 on, and the `OP_MustBeInt` in front of it from 3.40.0 on.
- The foreign key insert fails in all of these versions.

We found it with a property-based test generator for SQLite databases and queries. Joins on an INTEGER PRIMARY KEY lost this row when we added a WHERE term that is true for it, and only the plan with the added term had a Bloom filter.
