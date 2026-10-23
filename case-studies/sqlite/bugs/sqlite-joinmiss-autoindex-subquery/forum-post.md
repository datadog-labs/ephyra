**Title:** A correlated subquery over a three-way join counts 0 for later outer rows (JoinMiss deletes automatic-index entries)

```sql
CREATE TABLE t0(x INT);
CREATE TABLE t1(a INT, b INT);
CREATE TABLE t3(k INTEGER PRIMARY KEY);
INSERT INTO t0 VALUES(1);
INSERT INTO t1 VALUES(1,1);
INSERT INTO t3 VALUES(2);
SELECT y, (SELECT count(*) FROM t0 CROSS JOIN t1 CROSS JOIN t3
           WHERE t1.a = t0.x AND t3.k = t1.b + y)
  FROM (SELECT 0 AS y UNION ALL SELECT 1);
```

Output from trunk (check-in f69fd5f5c3, 2026-10-12 23:38), built from the source tarball with default options:

```
0|0
1|0
```

Expected `0|0` and `1|1`. For y=1 the one row of each table qualifies: t1.a = t0.x = 1, and t3.k = 2 = t1.b + 1. After `.testctrl optimizations -JoinMiss`, trunk gives `0|0` and `1|1`. So does trunk with `PRAGMA automatic_index=OFF`, and so does 3.54.0.

The CROSS JOINs only fix the join order. With ordinary joins the planner can choose the same plan by itself, for example after ANALYZE here:

```sql
CREATE TABLE t0(x INT);
CREATE TABLE t1(a INT, b INT);
CREATE TABLE t3(k INTEGER PRIMARY KEY);
CREATE TABLE o(y INT);
INSERT INTO t0 SELECT value%3 FROM generate_series(1,5);
INSERT INTO t1 SELECT value%5, value FROM generate_series(1,50);
INSERT INTO t3 SELECT value*2 FROM generate_series(1,20);
INSERT INTO o VALUES(0),(1),(0),(1);
ANALYZE;
SELECT y, (SELECT count(*) FROM t0, t1, t3 WHERE t1.a = t0.x AND t3.k = t1.b + y) FROM o;
-- trunk: 0|20 1|0 0|0 1|0
-- expected 20 for every row, as with -JoinMiss and in 3.54.0; the two y=0 rows already disagree
```

The plan is `SCAN t0`, `SEARCH t1 USING AUTOMATIC COVERING INDEX (a=?)`, `SEARCH t3 USING INTEGER PRIMARY KEY (rowid=?)`.

It is not limited to a scalar subquery in the result list. Over a table `ty(y, c)` holding y = 0 and 1, the same subquery gives wrong answers as EXISTS, NOT EXISTS, `2 IN (SELECT t3.k ...)`, a scalar subquery in WHERE, HAVING, ORDER BY, CASE or an ON clause, inside a view, in a recursive CTE (in the outer query and in the recursive step), as a window function's argument, in a RETURNING clause, in an upsert's `DO UPDATE SET n = (... t3.k = t1.b + excluded.id)`, and in a trigger body that runs the correlated subquery. It changes data too. `UPDATE ty SET c = (SELECT count(*) ...)` stores 0 for the y=1 row. `DELETE FROM ty WHERE EXISTS (...)` leaves the y=1 row in place, and `UPDATE ty SET c = 99 WHERE EXISTS (...)` skips it. The lookup on t3 fails the same way through an ordinary index, a DESC index, a two-column index or a WITHOUT ROWID primary key, with `IS` in place of `=`, with a fourth table between t1 and t3, and with `t9 RIGHT JOIN t0 ON t0.x = t9.z` in front of t1 and t3. The automatic index can also be on the outermost loop, with a key that uses only outer columns:

```sql
CREATE TABLE t1(a INT, b INT);
CREATE TABLE t2(c INT);
CREATE TABLE t3(k INTEGER PRIMARY KEY);
INSERT INTO t1 VALUES(1,1),(1,2);
INSERT INTO t2 VALUES(1);
INSERT INTO t3 VALUES(2);
CREATE TABLE o(y INT, z INT);
INSERT INTO o VALUES(1,5),(1,2);
SELECT z, (SELECT count(*) FROM (SELECT DISTINCT a, b FROM t1) AS v CROSS JOIN t2 CROSS JOIN t3
           WHERE v.a = y AND t3.k = z) FROM o;
-- trunk: 5|0 2|0, expected 5|0 2|2
```

These give the right answers on trunk:
- plain joins where the value comes from a table in the same FROM clause (`FROM ty CROSS JOIN t0 CROSS JOIN t1 CROSS JOIN t3 WHERE ... t3.k = t1.b + y`);
- a correlated subquery that uses the outer column only outside the lookup key (`t3.k = t1.b + 1 AND t3.k > y`);
- a trigger body whose key uses `new.y`, because every firing rebuilds the automatic index;
- one prepared statement re-run with a different bound parameter in the key (`t3.k = t1.b + ?`);
- a lookup on the right side of a LEFT JOIN.

We saw no crash or assertion failure in an `--enable-debug` build.

**Cause, as far as we can tell.** `findNoMatchContinuation()` (wherecode.c) returns `pParent->addrDeleteCont` when the inner-most outer loop that feeds the lookup key uses an automatic index and is the only loop in the key's mask:

```c
  mKey = 0;
  for(i=0; i<pLoop->nLTerm; i++){
    ...
    mKey |= pLoop->aLTerm[i]->prereqRight;
  }
  ...
  if( (pParent->pWLoop->wsFlags & WHERE_AUTO_INDEX)!=0
   && (mKey & (mKey-1))==0
  ){
```

`sqlite3WhereEnd()` then codes an `OP_Delete` of the current automatic-index entry ("delete useless auto-index entry"). That is safe only if the entry would miss again every time it is visited. But `prereqRight` has no bits for columns of an outer query, so `t3.k = t1.b + y` looks as if it depends on t1 alone. And `constructAutomaticIndex()` fills the index under `OP_Once`, so a correlated subquery fills it on its first run and keeps using it on every later run. The entry for t1's row is deleted while y=0 (key 1 has no match) and is gone when y=1 (key 2 would match). EXPLAIN of the first query shows this inside the subroutine for the subquery (explain comments from an `--enable-debug` build):

```
28   Once           0     42    0                 0
29   OpenAutoindex  4     3     0   k(3,B,,)      0   nColumn=3; for t1
...
51   Copy           2     17    0                 2   r[17]=r[2]
52   Add            17    11    16                0   r[16]=r[17]+r[11]
...
55   SeekRowid      3     58    16                0   intkey=r[16]
56   AggStep        0     0     9   count(0)      0   accum=r[9] step(r[0])
57   Goto           0     59    0                 0
58   Delete         4     0     0                 2   delete useless auto-index entry
59   Next           4     48    0                 0
```

With -JoinMiss the SeekRowid jumps to the `Next 4` and there is no Delete.

The patch below keeps the deletion only when the right-hand side of every key term depends on nothing but the automatic index's row. It uses `sqlite3ExprIsSingleTableConstraint()`, which also rejects non-deterministic functions. Every query above then gives the same answer as with -JoinMiss. The deletion is still coded for plain joins with keys like `t3.k = t1.b`, `t1.b + 1`, `t1.b + ?1` and `abs(t1.b)`, for a correlated subquery that uses the outer column only outside the key, and for a trigger body keyed on `new.y`:

```diff
--- a/src/wherecode.c
+++ b/src/wherecode.c
@@ -1616,15 +1616,32 @@
   ** is controlled by an automatic index and if the automatic index is
   ** the only outer loop that influences the key, then arrange to delete
   ** the automatic index entry if there is no match on the key.
+  **
+  ** The automatic index is filled only once per statement (OP_Once), but
+  ** this WHERE loop might be run many times, for example as part of a
+  ** correlated subquery.  mKey does not show references to columns of an
+  ** outer query, so also check that every key value depends on nothing
+  ** but the current row of the automatic index.  Otherwise an entry that
+  ** finds no match on one run might find a match on a later run.
   */
   pParent = &pWInfo->a[iParent];
   if( (pParent->pWLoop->wsFlags & WHERE_AUTO_INDEX)!=0
    && (mKey & (mKey-1))==0
   ){
-    if( pParent->addrDeleteCont==0 ){
-      pParent->addrDeleteCont = sqlite3VdbeMakeLabel(pWInfo->pParse);
+    for(i=0; i<pLoop->nLTerm; i++){
+      Expr *pX = pLoop->aLTerm[i]->pExpr;
+      assert( pX->op==TK_EQ || pX->op==TK_IS );
+      if( !sqlite3ExprIsSingleTableConstraint(pX->pRight, pTabList,
+                                              pParent->iFrom, 0) ){
+        break;
+      }
     }
-    return pParent->addrDeleteCont;
+    if( i>=pLoop->nLTerm ){
+      if( pParent->addrDeleteCont==0 ){
+        pParent->addrDeleteCont = sqlite3VdbeMakeLabel(pWInfo->pParse);
+      }
+      return pParent->addrDeleteCont;
+    }
   }
 
   testcase( iParent<iLevel-1 );
```

The veryquick tests give the same results with the patch as without it, in default and `--enable-debug` builds (25 errors in json101, mjournal, func, literal, json501 and types3, which our Tcl 8.5 environment also gives without the patch). autoindex*.test, where*.test, join*.test and subquery*.test pass with and without it. We also ran 3,000 generated correlated-subquery variants on a debug build with the patch (comma joins, views, EXISTS, IN and scalar forms, with and without ANALYZE), and all matched -JoinMiss. Still, take the patch as a pointer to the cause rather than a proposed fix.

**Versions** (built from the source tarball, or from the GitHub mirror's copy of each check-in, with default options): trunk e24a8fe986 (2026-10-09 21:05, the merge of the tree-tracker branch), dc388e83b0, a9d8cddccc and f69fd5f5c3 give the wrong answers. e24a8fe986's trunk parent fa41e62016, trunk 379c017e8f and 3.54.0 give the right ones, so no release is affected. On the tree-tracker branch the deletion is already in the first check-in, ead82958b3 (2026-09-29). There the query needs a fourth table between t1 and t3, because that version only took the long jump when it skipped a loop. The three-table query above fails from aae085c5ac (2026-09-30) on.

We found it while reading the new tree-tracker code and writing tests for it.
