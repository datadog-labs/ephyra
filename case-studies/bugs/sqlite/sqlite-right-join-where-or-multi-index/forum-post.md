A related case, on the RIGHT JOIN side of the same condition in `whereLoopAddOr()` that was changed for the LEFT JOIN report above. Here a WHERE-clause OR term drives a MULTI-INDEX OR on a table to the left of a RIGHT JOIN:

```sql
CREATE TABLE t0(a);
CREATE TABLE t1(a, b);
CREATE INDEX t1a ON t1(a);
CREATE TABLE t2(c);
INSERT INTO t0 VALUES(0);
INSERT INTO t1 VALUES(0, 1);
ANALYZE;
INSERT INTO t2 VALUES(9);
SELECT * FROM t0 JOIN t1 ON t1.a IN (t0.a) RIGHT JOIN t2 ON 1
 WHERE t1.b IS 5 OR t1.a IS 5 ORDER BY t1.b;
```

Output from trunk (check-in d6f1df8c0bbc, 2026-10-23), and also from 3.54.0, 3.53.4, 3.50.4, 3.40.0 and 3.39.0:

```
0|0|1|9
```

Expected: no rows, since neither `t1.b IS 5` nor `t1.a IS 5` is true. The OR term cannot be tested at t1's level (it has to wait for the RIGHT JOIN), so no sub-loop tests it, and `disableTerm()` then marks it as coded, so no later level tests it either.

The patch below adds one more clause to the same condition, so a table to the left of a RIGHT JOIN uses an OR term only when `constraintCompatibleWithOuterJoin()` accepts it. With it, the query returns no rows. The fix already on trunk does not cover this case, and this change does not cover the earlier one, so both clauses are needed:

```diff
--- a/src/where.c
+++ b/src/where.c
@@ -4866,6 +4866,8 @@
      && (pTerm->u.pOrInfo->indexable & pNew->maskSelf)!=0
      && (!ExprHasProperty(pTerm->pExpr, EP_OuterON)
           || pTerm->pExpr->w.iJoin==iCur)
+     && ((pItem->fg.jointype & JT_LTORJ)==0
+          || constraintCompatibleWithOuterJoin(pTerm, pItem))
     ){
       WhereClause * const pOrWC = &pTerm->u.pOrInfo->wc;
       WhereTerm * const pOrWCEnd = &pOrWC->a[pOrWC->nTerm];
```
