**Title:** Rows out of order despite ORDER BY x DESC in a query with GROUP BY x over a subquery sorted by x ASC

```sql
CREATE TABLE t(a);
INSERT INTO t VALUES(1),(2);
SELECT a FROM (SELECT a FROM t ORDER BY a LIMIT 2) GROUP BY a ORDER BY a DESC;
SELECT a FROM (SELECT a FROM t ORDER BY a LIMIT 2) GROUP BY a ORDER BY a DESC LIMIT 1;
```

Output from trunk (check-in dc388e83b05d, 2026-10-10 19:19), built from the source tarball with default options:

```
1
2
1
```

Expected `2`, `1` for the first query and `2` for the second. We are not relying on the subquery's ORDER BY to order the result: the outer query has its own `ORDER BY a DESC`, and that is what is not honored. With an outer LIMIT, the wrong row is returned. With the order-by-subquery optimization off (`.testctrl optimizations -OrderBySubq`), both queries give the expected result, and so does the same query without the GROUP BY or with DISTINCT in its place:

```sql
SELECT a FROM (SELECT a FROM t ORDER BY a LIMIT 2) ORDER BY a DESC;           -- 2, 1
SELECT DISTINCT a FROM (SELECT a FROM t ORDER BY a LIMIT 2) ORDER BY a DESC;  -- 2, 1
```

It needs an outer query with GROUP BY and ORDER BY on the same columns, over a subquery whose ORDER BY on those columns is kept (here because of the LIMIT; a MATERIALIZED CTE or a GROUP BY in the subquery keeps it too) but sorts the other way or puts NULLs at the other end. With a NULL in `t`, a subquery `ORDER BY a NULLS LAST` under an outer `GROUP BY a ORDER BY a` returns the NULL last. CTEs (MATERIALIZED or not), views, aggregates (`SELECT a, count(*) ... GROUP BY a ORDER BY a DESC`), a join with another table, and several columns (`GROUP BY a, b ORDER BY a, b DESC` over a subquery `ORDER BY a, b`) all fail in the same way. The result is correct when the directions match, when the outer ORDER BY has a NULLS FIRST/LAST other than the default, and when the subquery's ORDER BY is dropped (a plain subquery without LIMIT).

**Cause, as far as we can tell.** When the GROUP BY and ORDER BY terms are the same, select.c calls `sqlite3WhereBegin()` with `WHERE_GROUPBY|WHERE_SORTBYGROUP` and drops the ORDER BY sort if `sqlite3WhereIsSorted()` is true. `wherePathSolver()` sets `pWInfo->sorted` from a second, strict check, `wherePathSatisfiesOrderBy(pWInfo, pWInfo->pOrderBy, pFrom, 0, ...)`, which passes 0 as `wctrlFlags`. For a subquery loop, that check calls `wherePathMatchSubqueryOB()`, which tests the WhereInfo's flags instead of the argument:

```c
    if( (pWInfo->wctrlFlags & WHERE_GROUPBY)==0 ){
      u8 sfOB = pOrderBy->a[iOB].fg.sortFlags;   /* sortFlags for iOB */
```

`WHERE_GROUPBY` is set there, so the direction and NULLS checks are skipped and the path counts as sorted. The index path in `wherePathSatisfiesOrderBy()` tests the `wctrlFlags` argument instead. Passing the argument through makes every query above give the expected result:

```diff
--- a/src/where.c
+++ b/src/where.c
@@ -5086,5 +5086,6 @@
   ExprList *pOrderBy,     /* The ORDER BY clause on the whole query */
   Bitmask *pRevMask,      /* When loops need to go in reverse order */
-  Bitmask *pOBSat         /* Which terms of pOrderBy are satisfied so far */
+  Bitmask *pOBSat,        /* Which terms of pOrderBy are satisfied so far */
+  u16 wctrlFlags          /* WHERE_GROUPBY or _DISTINCTBY or _ORDERBY_LIMIT */
 ){
   int iOB;                /* Index into pOrderBy->a[] */
@@ -5104,5 +5105,5 @@
     if( pOBExpr->iTable!=iCur ) break;
     if( pOBExpr->iColumn!=pSubOB->a[jSub].u.x.iOrderByCol-1 ) break;
-    if( (pWInfo->wctrlFlags & WHERE_GROUPBY)==0 ){
+    if( (wctrlFlags & WHERE_GROUPBY)==0 ){
       u8 sfOB = pOrderBy->a[iOB].fg.sortFlags;   /* sortFlags for iOB */
       u8 sfSub = pSubOB->a[jSub].fg.sortFlags;   /* sortFlags for jSub */
@@ -5287,5 +5288,5 @@
          && OptimizationEnabled(db, SQLITE_OrderBySubq)
          &&  wherePathMatchSubqueryOB(pWInfo,pLoop,iLoop,iCur,
-                                     pOrderBy,pRevMask, &obSat)
+                                     pOrderBy,pRevMask, &obSat, wctrlFlags)
         ){
           nColumn = 0;
```

With it, a query whose directions match still has no ORDER BY sort. An opposite-direction query gets "USE TEMP B-TREE FOR ORDER BY", or for a MATERIALIZED CTE a backward scan. We have checked it against these queries and a few thousand generated ones of the same shape (compared with the optimization off), but have not run the test suite, so take it as a pointer to the cause rather than a proposed fix.

**Versions** (official amalgamations or tarballs, default options): 3.44.0, 3.45.0, 3.46.0 and 3.46.1 give `2, 1`. 3.47.0, the release that added the order-by-subquery optimization, gives `1, 2`, and so do 3.48.0, 3.49.0, 3.50.0, 3.51.0, 3.53.4, 3.54.0 and trunk dc388e83b05d.

We found it with a property-based test generator for SQLite databases and queries: a query whose ORDER BY fixes the row order completely returned its rows out of that order, and the same query with its joins written in another order returned them in a different order.
