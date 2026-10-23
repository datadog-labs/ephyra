**Title:** FTS5: wrong instances below a non-matching AND, and on the right of a NOT whose left side is at EOF

Two cases in `ext/fts5/fts5_expr.c` where `highlight()` (and `snippet()`, `bm25()`, `xInst()` etc.) gets the wrong instances for a matched row. The matched rows are right.

**Case 1:** a phrase below a non-root AND loses its instances for a later row.

```sql
CREATE VIRTUAL TABLE t1 USING fts5(a);
INSERT INTO t1(rowid, a) VALUES (1, 'x y'), (2, 'x z');
SELECT rowid, highlight(t1, 0, '[', ']') FROM t1('x AND (z OR "y x")');
SELECT rowid, highlight(t1, 0, '[', ']') FROM t1('(x AND (z OR "y x")) OR zz');
```

```
-- trunk:
2|[x] [z]
2|[x] z
-- expected:
2|[x] [z]
2|[x] [z]
```

In row 1 the AND fails ("y x" is not a phrase match) and `fts5ExprNodeTest_AND()` calls `fts5ExprNodeZeroPoslist()` on its whole subtree, which also empties "z", already advanced to row 2 by the OR. "z" is not re-tested when the AND reaches row 2.

**Case 2:** the right side of a NOT keeps its instances when the left side matches no row.

```sql
CREATE VIRTUAL TABLE t3 USING fts5(a);
INSERT INTO t3(rowid, a) VALUES (1, 'g b');
SELECT rowid, highlight(t3, 0, '[', ']') FROM t3('g OR (zz NOT b)');
INSERT INTO t3(rowid, a) VALUES (2, 'zz');
SELECT rowid, highlight(t3, 0, '[', ']') FROM t3('g OR (zz NOT b)');
```

```
-- trunk:
1|[g] [b]
1|[g] b
2|[zz]
-- expected:
1|[g] b
1|[g] b
2|[zz]
```

Row 1 should not depend on whether another row contains "zz". When the left side is at EOF after `fts5ExprNodeFirst()`, the NOT node is marked EOF there, so `fts5ExprNodeTest_NOT()` never runs the `fts5ExprNodeZeroPoslist(p2)` it does for a left side at EOF.

They are reported together because trunk's full clearing in case 1 hides case 2 when such a NOT is below an AND; the case 1 change alone breaks a query that trunk gets right:

```sql
CREATE VIRTUAL TABLE t4 USING fts5(a);
INSERT INTO t4(rowid, a) VALUES (1, 'y x'), (2, 'g b'), (3, 'x y');
SELECT rowid, highlight(t4, 0, '[', ']') FROM t4('g OR ("x y" AND (y OR (zz NOT b)))');
-- trunk and the full patch: 2|[g] b    3|[x y]
-- case 1 change only:       2|[g] [b]  3|[x y]
```

Seen on trunk d6f1df8c0bbc (2026-10-23) and 3.51.0. This is separate from the NEAR report (https://sqlite.org/bugs/forumpost/9fa08e2dbd); the patches touch different functions and apply in either order.

The patch below makes the AND empty only phrases whose nodes are on the AND's rowid, and empties the NOT's right side in `fts5ExprNodeFirst()` when the NOT is at EOF:

```diff
--- a/ext/fts5/fts5_expr.c
+++ b/ext/fts5/fts5_expr.c
@@ -1052,6 +1052,24 @@
     int i;
     for(i=0; i<pNode->nChild; i++){
       fts5ExprNodeZeroPoslist(pNode->apChild[i]);
+    }
+  }
+}
+
+/*
+** Zero the position lists of the phrases in the subtree rooted at pNode
+** that belong to nodes currently pointing to rowid iRowid. Nodes that
+** point to a different rowid have already been advanced past iRowid
+** (for example the children of an OR node that do not match iRowid),
+** so their position lists belong to that later row and are left as is.
+*/
+static void fts5ExprNodeZeroPoslistAt(Fts5ExprNode *pNode, i64 iRowid){
+  if( pNode->eType==FTS5_STRING || pNode->eType==FTS5_TERM ){
+    if( pNode->iRowid==iRowid ) fts5ExprNodeZeroPoslist(pNode);
+  }else{
+    int i;
+    for(i=0; i<pNode->nChild; i++){
+      fts5ExprNodeZeroPoslistAt(pNode->apChild[i], iRowid);
     }
   }
 }
@@ -1375,7 +1393,7 @@
   }while( bMatch==0 );
 
   if( pAnd->bNomatch && pAnd!=pExpr->pRoot ){
-    fts5ExprNodeZeroPoslist(pAnd);
+    fts5ExprNodeZeroPoslistAt(pAnd, iLast);
   }
   pAnd->iRowid = iLast;
   return SQLITE_OK;
@@ -1523,6 +1541,12 @@
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
