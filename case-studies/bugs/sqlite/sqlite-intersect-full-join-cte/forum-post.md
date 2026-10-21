**Title:** Rows missing when a UNION ALL subquery or CTE is joined with an inner join followed by a RIGHT or FULL JOIN

```sql
CREATE TABLE t1(a);
INSERT INTO t1 VALUES(1);
SELECT z.a, v FROM t1 AS x JOIN t1 AS y ON 0 RIGHT JOIN t1 AS z
  CROSS JOIN (SELECT 2 AS v FROM t1 UNION ALL SELECT 3 FROM t1);
```

Output from trunk (check-in dc388e83b05d, 2026-10-10 19:19), built from the source tarball with default options:

```
1|3
```

Expected `1|2` and `1|3`. `x JOIN y ON 0` is empty, so the RIGHT JOIN gives z's row NULL-extended, and crossing it with the subquery's two rows gives two rows. SQLite agrees in other forms of the same query: `SELECT count(*)` over the same FROM clause gives 2, and with `.testctrl optimizations -FlttnUnionAll` (or `-QueryFlattener`) the query returns both rows.

Only the last SELECT of the compound keeps its row: with `UNION ALL SELECT 4 FROM t1` added, the result is just `1|4`. The result is also wrong with FULL JOIN in place of RIGHT JOIN, with `ON NULL` or a column term such as `ON x.a=y.a+1` in place of `ON 0`, with a view or a CTE (default or NOT MATERIALIZED) in place of the subquery, and with the three right-join tables inside a subquery or CTE that is crossed with the compound. It is correct with `AS MATERIALIZED`, with UNION in place of UNION ALL, when an arm of the compound has no FROM (`SELECT 2 AS v FROM t1 UNION ALL SELECT 3`), with DISTINCT or an aggregate in the outer query, and with `x RIGHT JOIN z ON 0` alone, without the inner join.

The RIGHT JOIN can also be inside the compound. This is the shape we first hit, a CTE joined with itself:

```sql
WITH v AS (SELECT a FROM t1 UNION ALL
           SELECT z.a FROM t1 AS x JOIN t1 AS y ON 0 RIGHT JOIN t1 AS z)
SELECT * FROM v AS p, v AS q;
```

`SELECT * FROM v` returns two rows and `SELECT count(*) FROM v AS p, v AS q` returns 4, but this query returns two rows (`1|1` twice).

**Cause, as far as we can tell.** For each arm except the last, `flattenSubquery()` copies the outer query with `sqlite3SelectDup()` and gives the copy new cursor numbers with `renumberCursors()`. Its callback remaps `w.iJoin` only for `EP_OuterON` terms:

```c
static int renumberCursorsCb(Walker *pWalker, Expr *pExpr){
  int op = pExpr->op;
  if( op==TK_COLUMN || op==TK_IF_NULL_ROW ){
    renumberCursorDoMapping(pWalker, &pExpr->iTable);
  }
  if( ExprHasProperty(pExpr, EP_OuterON) ){
    renumberCursorDoMapping(pWalker, &pExpr->w.iJoin);
  }
  return WRC_Continue;
}
```

The inner join's `ON 0` has been moved to the WHERE clause as an `EP_InnerON` term with `w.iJoin` set to y's cursor, so in the copy it still names y's old cursor. In `exprAnalyze()`, `sqlite3WhereGetMask()` returns 0 for that cursor, `(prereqAll>>1)>=x` holds, and `EP_InnerON` is cleared. The term is then an ordinary WHERE term. `ON 0` is coded as a jump past the whole loop, which skips the RIGHT JOIN's pass over unmatched rows too. A column term is applied to the NULL-extended row and rejects it.

Remapping `w.iJoin` for both flags fixes it:

```diff
--- a/src/select.c
+++ b/src/select.c
@@ -4124,7 +4124,7 @@
   if( op==TK_COLUMN || op==TK_IF_NULL_ROW ){
     renumberCursorDoMapping(pWalker, &pExpr->iTable);
   }
-  if( ExprHasProperty(pExpr, EP_OuterON) ){
+  if( ExprHasProperty(pExpr, EP_OuterON|EP_InnerON) ){
     renumberCursorDoMapping(pWalker, &pExpr->w.iJoin);
   }
   return WRC_Continue;
```

With this change, every failing query above returns the expected rows, the correct ones are unchanged, and the veryquick tests show no new failures. That is all we checked, so take it as a pointer to the cause rather than a proposed fix.

The callback has been this way since 3.39.0. Before check-in e0beda7ffe (for forum post ccfb3b5052), the same branch of `exprAnalyze()` raised "ON clause references tables to its right" when the query had a RIGHT or FULL JOIN. So up to 3.50.4 the first query failed with that error, and the change above makes 3.50.4 return the right rows as well.

**Versions** (official amalgamations or tarballs, default options), first query: 3.39.0, 3.39.4, 3.40.0, 3.40.1, 3.41.0 through 3.50.0 (each x.0 release), 3.45.3, 3.46.1 and 3.50.4 fail with "ON clause references tables to its right". 3.51.0, 3.52.0, 3.53.4, 3.54.0 and trunk dc388e83b05d return `1|3`. The CTE query gives the same error on 3.41.0 to 3.50.4, and a single row on 3.39 and 3.40. 3.54.0 and trunk already include check-in cfa69ea48a, the fix for the 2026-10-08 report on flattening a UNION ALL subquery with a RIGHT JOIN, so this is a separate problem.

We found it with a property-based test generator for SQLite databases and queries: on a CTE joined with itself, `SELECT DISTINCT * ... WHERE 1` returned four rows, while the same query written as `SELECT * ... WHERE 1 INTERSECT SELECT * ...` returned two.
