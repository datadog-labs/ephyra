**Title:** FTS5: the right operand of a NOT reports instances when the left operand matches no row

```sql
CREATE VIRTUAL TABLE t1 USING fts5(a);
INSERT INTO t1(rowid, a) VALUES (1, 'g b');
SELECT rowid, highlight(t1, 0, '[', ']') FROM t1('g OR (zz NOT b)');
SELECT rowid, highlight(t1, 0, '[', ']') FROM t1('g OR (zz AND b)');
INSERT INTO t1(rowid, a) VALUES (2, 'zz');
SELECT rowid, highlight(t1, 0, '[', ']') FROM t1('g OR (zz NOT b)');
```

Output from trunk (check-in 3769f60be38b, 2026-10-21 16:05), built from the source tarball with --enable-fts5. 3.51.0 gives the same rows:

```
1|[g] [b]
1|[g] b
1|[g] b
2|[zz]
```

Expected:

```
1|[g] b
1|[g] b
1|[g] b
2|[zz]
```

`zz NOT b` matches no row, so row 1 is matched only by "g". The right operand of a NOT never reports instances otherwise: the same query run again after a row containing "zz" is added (the third query) does not highlight "b" in row 1, and neither does `g OR (zz AND b)`. The highlight of row 1 should not depend on whether some other row contains "zz". `bm25()` is affected too: for the first query it is -2.0e-06, against -1.0e-06 for the query `g`, so `ORDER BY rank` can order rows wrongly. So is everything else that reads instances: `snippet()` and the `xInstCount()`, `xInst()`, `xPhraseFirst()` and `xPhraseFirstColumn()` APIs of custom auxiliary functions. It is the same with detail=column and detail=none tables, except that there `ORDER BY rank` gives `[g] b`, because the sorter path rebuilds the instances through `fts5ExprCheckPoslists()`, which clears a NOT that does not match.

It happens when the left operand of the NOT is at EOF from the start: a term that is in no row, a phrase or NEAR group whose terms never occur together in one row, or nested column filters that leave no columns, as in `a : (c : h)`. If the left operand has rows but does not match the current one, the right operand is cleared as expected.

The cause is in `ext/fts5/fts5_expr.c`. `fts5ExprNodeTest_NOT()` empties the position lists of the right operand when the left operand reaches EOF (`if( p1->bEof ) fts5ExprNodeZeroPoslist(p2);`). But when the left operand is already at EOF after `fts5ExprNodeFirst()` has initialized the children, the NOT node is marked EOF there, and the `fts5ExprNodeTest()` call that follows does nothing for a node at EOF. So `fts5ExprNodeTest_NOT()` never runs, and the right operand keeps its position list for the first row it points to. Its node is not at EOF, so `sqlite3Fts5ExprPoslist()` returns that list when the query reaches the row through another operand. For detail=column and detail=none, the nonempty list makes `sqlite3Fts5ExprClearPoslists()` treat the phrase as present in the row.

The patch below empties the position lists of the right operand in `fts5ExprNodeFirst()` when the NOT node is at EOF, as `fts5ExprNodeTest_NOT()` would:

```diff
--- a/ext/fts5/fts5_expr.c
+++ b/ext/fts5/fts5_expr.c
@@ -1523,6 +1523,12 @@
       default:
         assert( pNode->eType==FTS5_NOT );
         pNode->bEof = pNode->apChild[0]->bEof;
+        if( pNode->bEof ){
+          /* The NOT node is at EOF, so fts5ExprNodeTest() below does not
+          ** call fts5ExprNodeTest_NOT(). Zero the position lists of the
+          ** phrases on the right-hand side here instead, as it would. */
+          fts5ExprNodeZeroPoslist(pNode->apChild[1]);
+        }
         break;
     }
   }
```

**Versions:** wrong in 3.46.0, 3.50.2, 3.51.0 (the macOS /usr/bin/sqlite3), 3.53.2 and trunk.

This is separate from the earlier report about NEAR groups that do not match a row. The two patches change different functions and apply in either order.

We found it with a randomized differential of FTS5 auxiliary-function output, written while checking the fix for that NEAR report.
