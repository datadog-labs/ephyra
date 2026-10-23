**Title:** a IN (...) AND c GLOB 'A' with an index on (a,c): GROUP BY splits a group, DISTINCT repeats a row (BLOB in c)

```sql
CREATE TABLE t(a, c);
CREATE INDEX ti ON t(a, c);
INSERT INTO t VALUES(1, 'A'), (2, 'A'), (1, x'41');
SELECT a, count(*) FROM t WHERE a IN (1,2) AND c GLOB 'A' GROUP BY a;
```

Output from trunk (check-in 74a6182a0c, 2026-10-13 18:36), built from the source tarball with default options. 3.54.0 and 3.53.4 give the same:

```
1|1
2|1
1|1
```

Expected:

```
1|2
2|1
```

All three rows match: `x'41' GLOB 'A'` is 1, because LIKE and GLOB match BLOBs unless SQLite is built with SQLITE_LIKE_DOESNT_MATCH_BLOBS. So group a=1 has two rows, and it should come out once. With `+a IN (1,2)` in place of `a IN (1,2)`, trunk gives `1|2` and `2|1`. So does `a = 1` on its own (`1|2`).

The plan is `SEARCH t USING COVERING INDEX ti (a=? AND c>? AND c<?)` with no temp b-tree, so the planner relies on the index order for the GROUP BY. Other forms over the same table give wrong answers too:

```sql
SELECT DISTINCT a FROM t WHERE a IN (1,2) AND c GLOB 'A';
-- 1 2 1, expected 1 2
SELECT a, quote(c) FROM t WHERE a IN (1,2) AND c GLOB 'A' ORDER BY a;
-- 1|'A' 2|'A' 1|X'41', not ordered by a
SELECT a, quote(c) FROM t WHERE a IN (1,2) AND c GLOB 'A' ORDER BY a DESC;
-- 1|X'41' 2|'A' 1|'A', not ordered by a
SELECT a, quote(c) FROM (SELECT a, c FROM t WHERE a IN (1,2) AND c GLOB 'A'
                         INTERSECT SELECT a, c FROM t);
-- 1|'A' 2|'A', expected the three rows; the (1, x'41') row is dropped
```

The same happens with `c GLOB 'A*'`, with `a IN (SELECT 1 UNION SELECT 2)`, with `(a=1 OR a=2)`, with `PRAGMA case_sensitive_like=1` and `c LIKE 'A'`, and with `c LIKE 'a'` when c is declared `COLLATE NOCASE`. It does not happen with `a = 1`, with `+a IN (1,2)`, or with the range written out as `c >= 'A' AND c < 'B'` (that range does not include BLOBs, and the answer `1|1 2|1` is right for it).

No IN is needed when the planner picks a skip-scan on a:

```sql
CREATE TABLE t(a, c);
CREATE INDEX ti ON t(a, c);
WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<100)
INSERT INTO t SELECT i%3, 'x'||i FROM n;
INSERT INTO t VALUES(2, 'A'), (1, x'41');
ANALYZE;
SELECT a, quote(c) FROM t WHERE c GLOB 'A' ORDER BY a;
-- 2|'A' 1|X'41', expected 1|X'41' 2|'A' (as with ORDER BY +a)
```

The plan is `SEARCH t USING COVERING INDEX ti (ANY(a) AND c>? AND c<?)`, again with no sorter.

**Cause, as far as we can tell.** For a LIKE or GLOB pattern with a fixed prefix, the LIKE optimization scans the index range twice: once over TEXT values and then again over BLOB values. In `sqlite3WhereCodeOneLoopStart()` (wherecode.c), the start of that "LIKE loop" (`pLevel->addrLikeRep`, with the counter `iLikeRepCntr`) is coded before `codeAllEqualityTerms()`, which codes the IN loops and the skip-scan seek on the earlier index columns. `sqlite3WhereEnd()` closes the IN loops and the skip-scan first and the LIKE loop last:

```c
    if( pLevel->addrSkip ){
      sqlite3VdbeGoto(v, pLevel->addrSkip);
      ...
    }
#ifndef SQLITE_LIKE_DOESNT_MATCH_BLOBS
    if( pLevel->addrLikeRep ){
      sqlite3VdbeAddOp2(v, OP_DecrJumpZero, (int)(pLevel->iLikeRepCntr>>1),
                        pLevel->addrLikeRep);
```

So the LIKE loop is the outermost of them, and the rows come out in two sorted runs: all values of a for the TEXT pass, then all values of a for the BLOB pass. EXPLAIN of the first query shows this:

```
6     Integer        1     7     0                    0   r[7]=1; LIKE loop counter
...
18    Rewind         3     41    0                    0
19      Column         3     0     8                    0   r[8]= cursor 3 column 0
...
22      SeekGE         2     40    8     2              0   key=r[8..9]
...
39      Next           2     24    0                    0
40    Next           3     19    0                    0
41    DecrJumpZero   7     7     0                    0   if (--r[7])==0 goto 7
```

(cursor 3 is the IN list, cursor 2 the index). `wherePathSatisfiesOrderBy()` (where.c) still treats the IN and skip-scan columns as ordered, so the GROUP BY, DISTINCT, ORDER BY and the INTERSECT merge all get rows that are not in the order they assume. With a single `a = ?` there is only one run per pass, so the order holds.

The patch below makes `wherePathSatisfiesOrderBy()` give up on such a loop, the same way it does for an index with `bUnordered`. With it, every query above gives the expected answer, and the plans gain `USE TEMP B-TREE FOR GROUP BY` / `FOR ORDER BY`. The plans for `a = 1 AND c GLOB 'A'` and for the explicit `c >= 'A' AND c < 'B'` range do not change. like.test, like2.test, like3.test, skipscan1/2/3/5/6.test, orderby1/2/5/9.test, in.test, in4/5/6.test, distinct.test, where.test, where2.test, where7.test, whereG.test, select1.test, select4.test, e_select.test and minmax4.test pass with it. Another way would be to move the LIKE loop inside the IN and skip-scan loops, which would keep the sort avoidance. Either way, take the patch as a pointer to the cause rather than a proposed fix.

```diff
--- src/where.c.orig
+++ src/where.c
@@ -5131,6 +5131,28 @@
 }
 
 /*
+** Return true if pLoop uses the LIKE optimization range (which is
+** scanned twice: once for TEXT and once for BLOB values) and one of the
+** equality-prefix columns is iterated by a skip-scan or an IN operator.
+** In that case the rows do not come out in index order.
+*/
+static int whereLoopLikeRepBreaksOrder(WhereLoop *pLoop){
+#ifndef SQLITE_LIKE_DOESNT_MATCH_BLOBS
+  int j;
+  u16 nEq = pLoop->u.btree.nEq;
+  if( (pLoop->wsFlags & WHERE_TOP_LIMIT)==0 ) return 0;
+  if( nEq>=pLoop->nLTerm ) return 0;
+  if( pLoop->aLTerm[nEq]==0
+   || (pLoop->aLTerm[nEq]->wtFlags & TERM_LIKEOPT)==0 ) return 0;
+  if( pLoop->nSkip>0 ) return 1;
+  for(j=0; j<nEq; j++){
+    if( pLoop->aLTerm[j] && (pLoop->aLTerm[j]->eOperator & WO_IN) ) return 1;
+  }
+#endif
+  return 0;
+}
+
+/*
 ** Examine a WherePath (with the addition of the extra WhereLoop of the 6th
 ** parameters) to see if it outputs rows in the requested ORDER BY
 ** (or GROUP BY) without requiring a separate sort operation.  Return N:
@@ -5296,6 +5318,11 @@
         pIndex = 0;
         nKeyCol = 0;
       }else if( (pIndex = pLoop->u.btree.pIndex)==0 || pIndex->bUnordered ){
+        return 0;
+      }else if( whereLoopLikeRepBreaksOrder(pLoop) ){
+        /* The LIKE optimization scans the range twice (TEXT, then BLOB)
+        ** in a loop that is outside of any skip-scan or IN loops on the
+        ** prefix columns, so the output is not ordered by the index. */
         return 0;
       }else{
         nKeyCol = pIndex->nKeyCol;
```

**Versions** (official amalgamations or the source tarball, default options): 3.7.17 to 3.15.0 (all of 3.7.17, 3.8.0-3.8.4, 3.8.7, 3.8.10, 3.9.0, 3.10.0, 3.11.0, 3.13.0, 3.14.0, 3.15.0) give the right answers for the IN query. Their plan is `SEARCH ... (a=?)`, without the LIKE range. 3.20.0, 3.22.0, 3.24.0 and every release we tried from there to 3.54.0, plus trunk, give `1|1 2|1 1|1`. We had no build between 3.15.0 and 3.20.0, so we have not narrowed down where it starts. The skip-scan query is wrong from 3.20.0 on too. 3.8.0 to 3.15.0 do not pick a skip-scan for it and give the right order. (On 3.20.0 the INTERSECT query happens to return all three rows, but GROUP BY, DISTINCT and ORDER BY are already wrong there.)

We found it with a property-based test generator for SQLite databases and queries. A SELECT DISTINCT query returned three rows, but the same query written as an INTERSECT returned two. The INTERSECT's left side used a skip-scan with the LIKE optimization, and the planner believed its output was already sorted.
