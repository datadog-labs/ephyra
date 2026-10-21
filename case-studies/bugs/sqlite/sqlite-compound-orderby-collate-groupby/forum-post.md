**Title:** Compound SELECT with ORDER BY ... COLLATE moves the last SELECT's GROUP BY, HAVING and DISTINCT to the whole compound

```sql
CREATE TABLE t1(a);
INSERT INTO t1 VALUES('x'), ('y'), ('y');
CREATE TABLE t2(a, b);
INSERT INTO t2 VALUES('x', 1), ('z', 2), ('z', 3);
SELECT a FROM t1 UNION SELECT a FROM t2 GROUP BY a HAVING count(*)>1
 ORDER BY a COLLATE NOCASE;
SELECT a FROM t1 UNION SELECT b FROM t2 GROUP BY b ORDER BY 1 COLLATE NOCASE;
SELECT 1 UNION SELECT 2 UNION ALL SELECT DISTINCT 1 ORDER BY 1 COLLATE NOCASE;
```

Output from trunk (check-in 69d51c35426f, 2026-10-16 20:31), built from the source tarball with default options. 3.54.0 gives the same:

```
Parse error near line 7: no such column: b
  SELECT a FROM t1 UNION SELECT b FROM t2 GROUP BY b ORDER BY 1 COLLATE NOCASE;
                                     error here ---^
1
2
```

Expected:

```
x
y
z
1
2
3
x
y
1
1
2
```

The first query returns no rows: `SELECT a FROM t2 GROUP BY a HAVING count(*)>1` is `z`, so the UNION should be x, y, z. Without the COLLATE (`ORDER BY a`, or `ORDER BY 1`) all three queries are right. EXCEPT behaves the same way (`... EXCEPT SELECT a FROM t2 GROUP BY a HAVING count(*)>1 ORDER BY a COLLATE NOCASE` gives no rows, expected x, y), and so does an aggregate with GROUP BY and no HAVING: `SELECT a, 0 FROM t1 UNION SELECT a, count(*) FROM t2 GROUP BY a ORDER BY 1 COLLATE NOCASE` gives `x|0`, `y|0` instead of `x|0`, `x|1`, `y|0`, `z|2`. The COLLATE need not be at the top of the ORDER BY term: `SELECT (a COLLATE NOCASE) || '' FROM t1 UNION SELECT a FROM t2 GROUP BY a HAVING count(*)>1 ORDER BY (a COLLATE NOCASE) || ''` fails with "no such column: a".

**Cause, as far as we can tell.** `convertCompoundSelectToSubquery()` (select.c) rewrites a UNION, EXCEPT or INTERSECT whose ORDER BY has a COLLATE as `SELECT * FROM (<compound>) ORDER BY ...`. It copies the last SELECT `p` into `pNew`, which becomes the last SELECT of the subquery, and turns `p` into the outer query. It then clears `p->pWhere` (the WHERE stays with `pNew`, right), but for GROUP BY and HAVING it clears them on `pNew` instead of `p`, so they leave the last SELECT and apply to the whole compound. EXPLAIN QUERY PLAN of the first query shows it: the RIGHT arm of the MERGE (UNION) has no `USE TEMP B-TREE FOR GROUP BY` (with the change below it has one). When the GROUP BY or HAVING names a column the compound's result does not have, that is the "no such column" error; otherwise the result is wrong. `SF_Distinct` likewise stays in `p->selFlags`, which matters only when the last operator is UNION ALL (the third query). Clearing them on `p` fixes every query above:

```diff
--- a/src/select.c
+++ b/src/select.c
@@ -5666,8 +5666,8 @@
   p->pEList = sqlite3ExprListAppend(pParse, 0, sqlite3Expr(db, TK_ASTERISK, 0));
   p->op = TK_SELECT;
   p->pWhere = 0;
-  pNew->pGroupBy = 0;
-  pNew->pHaving = 0;
+  p->pGroupBy = 0;
+  p->pHaving = 0;
   pNew->pOrderBy = 0;
   p->pPrior = 0;
   p->pNext = 0;
@@ -5675,7 +5675,7 @@
 #ifndef SQLITE_OMIT_WINDOWFUNC
   p->pWinDefn = 0;
 #endif
-  p->selFlags &= ~(u32)SF_Compound;
+  p->selFlags &= ~(u32)(SF_Compound|SF_Distinct);
   assert( (p->selFlags & SF_Converted)==0 );
   p->selFlags |= SF_Converted;
   assert( pNew->pPrior!=0 );
```

With it, the select*, resolver*, e_select*, collate*, tkt-* and window* tests pass as before, and veryquick shows no new failures. Take it as a pointer to the cause rather than a proposed fix.

**Versions** (official builds, default options): 3.20.0, 3.25.0, 3.30.0, 3.35.0, 3.40.0, 3.44.0, 3.50.4, 3.51.0, 3.53.4, 3.54.0 and trunk all give the outputs above (3.20.0 reports the error as "Error: near line 7: no such column: b").

We found it with a property-based test generator for SQLite databases and queries: an EXCEPT whose second SELECT had `GROUP BY ... HAVING <aggregate>`, ordered by a result column's expression that contained `COLLATE NOCASE`, failed with "no such column" on a column of its own FROM.
