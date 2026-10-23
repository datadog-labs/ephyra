**Title:** Multi-index OR on a WITHOUT ROWID table misses rows when an OR-term uses OP_SeekScan

```sql
CREATE TABLE t1(a, b, PRIMARY KEY(a, b)) WITHOUT ROWID;
INSERT INTO t1 VALUES(1, 1), (2, 1), (3, 1), (4, 1);
ANALYZE;
SELECT * FROM t1 WHERE a = 2 OR (a = 1 AND b IN (1, 2));
```

Output:

```
2|1
```

Expected (the row (1, 1) matches the second OR-term):

```
2|1
1|1
```

In `sqlite3WhereCodeOneLoopStart()`, the `OP_NullRow` that `OP_SeekScan` relies on is emitted only when `iLevel>0`, but the sub-loops of a multi-index OR are at level 0 of a nested `sqlite3WhereBegin()`. On a WITHOUT ROWID table they share the table cursor, so the `a = 2` sub-loop leaves it on (3, 1), and `OP_SeekScan` then skips both IN keys.

The patch below also emits the `OP_NullRow` in the sub-loops of a multi-index OR:

```diff
--- a/src/wherecode.c
+++ b/src/wherecode.c
@@ -2146,9 +2146,14 @@
       SWAP(u8, nBtm, nTop);
     }
 
-    if( iLevel>0 && (pLoop->wsFlags & WHERE_IN_SEEKSCAN)!=0 ){
+    if( (iLevel>0 || (pWInfo->wctrlFlags & WHERE_OR_SUBCLAUSE)!=0)
+     && (pLoop->wsFlags & WHERE_IN_SEEKSCAN)!=0
+    ){
       /* In case OP_SeekScan is used, ensure that the index cursor does not
-      ** point to a valid row for the first iteration of this loop. */
+      ** point to a valid row for the first iteration of this loop. This
+      ** is also required for the sub-loops of a multi-index OR, as the
+      ** index cursor may have been left pointing to a row by an earlier
+      ** OR-term or by an earlier iteration of an outer loop. */
       sqlite3VdbeAddOp1(v, OP_NullRow, iIdxCur);
     }
 
```

**Versions:** trunk d6f1df8c0bbc (2026-10-23) and 3.35.0 through 3.54.0. 3.33.0 gives the expected output.
