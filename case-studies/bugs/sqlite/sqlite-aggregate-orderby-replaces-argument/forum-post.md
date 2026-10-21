**Title:** sum(b ORDER BY a) FROM t1, t2 sums a instead of b: an aggregate's ORDER BY term replaces its argument

```sql
CREATE TABLE t1(a);
CREATE TABLE t2(b);
INSERT INTO t1 VALUES(42);
INSERT INTO t2 VALUES(2);
SELECT sum(b ORDER BY a) FROM t1, t2;
```

Output from trunk (check-in a9d8cddccc77, 2026-10-12 12:51), built from the source tarball with default options. 3.54.0 and 3.53.4 give the same:

```
42
```

Expected `2`; 42 is the value of `a`. The ORDER BY inside an aggregate only orders the aggregate's inputs. It does not change which values are aggregated, and the only value of `b` is 2. `sum(b)` returns 2. So does `sum(b ORDER BY a) FROM t2, t1` (the tables swapped), and so does `sum(b ORDER BY +a) FROM t1, t2`. `.testctrl optimizations -All` does not change the result.

The aggregate gets the ORDER BY term's value from the same row in place of its argument. Two more examples:

```sql
CREATE TABLE t3(c0, c1);
INSERT INTO t3 VALUES(1, 2);
SELECT count(a1.c1 ORDER BY a0.c1) FROM t3 AS a0 LEFT JOIN t3 AS a1 ON NULL;
-- 1, expected 0: a1.c1 is NULL in the only row, but a0.c1 (2) is counted

CREATE TABLE t4(k, v);
INSERT INTO t4 VALUES(1, 'x'), (2, 'y');
SELECT group_concat(q.v ORDER BY p.v) FROM t4 AS p, t4 AS q WHERE p.k<>q.k;
-- x,y, expected y,x: the rows are (p.v='x', q.v='y') and (p.v='y', q.v='x')
```

The result is wrong when all of these hold:
- the aggregate has one argument and its ORDER BY has one term;
- the two are the same expression, except that a column in the ORDER BY term comes from another table (or another alias of the same table) and has the same column number in its table: `b ORDER BY a`, `t2.y ORDER BY t1.y`, `abs(b) ORDER BY abs(a)`, `b*10 ORDER BY a*10`, `t2.rowid ORDER BY t1.rowid`;
- the ORDER BY term's table is the one that gets VDBE cursor 0. In a plain SELECT, that is the first table in FROM.

It happens with count, sum, total, avg, group_concat with one argument, json_group_array, median, count(DISTINCT ...) and with DESC or NULLS LAST, with or without GROUP BY. It also happens with inner, LEFT, RIGHT and FULL joins, in a correlated subquery whose ORDER BY term is a column of the outer query's first table, in INSERT ... SELECT, and when the first FROM item is a subquery that is not flattened. With GROUP BY, the per-group results are wrong: `SELECT a0.c0, count(a1.c1 ORDER BY a0.c1) FROM t0 AS a0, t0 AS a1 GROUP BY a0.c0` over the rows (NULL, NULL) and ('', 0) gives 0 and 2 where both groups should be 1. Not affected: min() and max() (their ORDER BY is ignored), aggregates with two arguments (group_concat(x, sep), string_agg, json_group_object, percentile), more than one ORDER BY term, a COLLATE on the ORDER BY term that the argument does not also have, and an ORDER BY term over any table other than the cursor-0 one. Also not affected: the same query with the first table, or the whole join, read through a view or CTE that is flattened, since cursor 0 then belongs to something else. A view or CTE over the other table does not help, and a first FROM item that is not flattened (a view with LIMIT, a MATERIALIZED CTE) is affected like the subquery above.

**Cause, as far as we can tell.** In `analyzeAggregate()` (expr.c), the code that sets up the ORDER BY sorter checks whether the single ORDER BY term is the same as the single argument. If it is, the sorter stores only the key (`bOBPayload = 0`), and `finalizeAggFunctions()` feeds the key column to the aggregate. The check is

```c
               && sqlite3ExprCompare(0,pOBList->a[0].pExpr,
                               pExpr->x.pList->a[0].pExpr,0)==0
```

with `iTab` set to 0. `sqlite3ExprCompare()` ignores a difference in `Expr.iTable` when `pA->iTable==iTab`. So an ORDER BY column on cursor 0 compares equal to the argument's column of the same number on any other cursor, and the argument is never stored. EXPLAIN for the first query shows the ORDER BY table opened with key columns only (`OpenEphemeral 2 2`, a `MakeRecord` of the key and the sequence number, then `Column 2 0` into `AggStep`). The aggregate-dedup call earlier in `analyzeAggregate()` passes -1. Passing -1 here too fixes every query above:

```diff
--- a/src/expr.c
+++ b/src/expr.c
@@ -7636,7 +7636,7 @@
               if( pOBList->nExpr==1
                && nArg==1
                && sqlite3ExprCompare(0,pOBList->a[0].pExpr,
-                               pExpr->x.pList->a[0].pExpr,0)==0
+                               pExpr->x.pList->a[0].pExpr,-1)==0
               ){
                 pItem->bOBPayload = 0;
                 pItem->bOBUnique = ExprHasProperty(pExpr, EP_Distinct);
```

When the argument and the ORDER BY term really are the same (`group_concat(b ORDER BY b)`, `count(DISTINCT b ORDER BY b)`), the patched build generates the same bytecode as before. The veryquick tests give the same results with the patch as without it (25 errors in json101, mjournal, func, literal, json501 and types3, which our Tcl 8.5 environment also gives without the patch), and aggorderby.test, aggnested.test and distinctagg.test pass. We also ran 16,000 generated aggregate-ORDER-BY queries over 2- and 3-way joins on a debug build with the patch. Each query matched the same query with `+` in front of the ORDER BY term, on the patched build and on unpatched trunk. The only exceptions were two `DISTINCT x ORDER BY x` queries that kept 0 rather than 0.0 out of equal values, which happens without the patch too. Still, take it as a pointer to the cause rather than a proposed fix.

**Versions** (official amalgamations or the source tarball, default options): 3.43.0 rejects the syntax. Every version we tried since aggregate ORDER BY arrived gives 42: 3.44.0, 3.44.1, 3.44.2, 3.45.0 to 3.45.3, 3.46.0, 3.46.1, 3.47.0, 3.48.0, 3.49.0, 3.50.0, 3.51.0, 3.51.3, 3.52.0, 3.53.0, 3.53.4, 3.54.0 and trunk a9d8cddccc77. 3.44.0's expr.c already has the `,0)==0` comparison.

We found it with a property-based test generator for SQLite databases and queries. It compared `count(a1.c1 ORDER BY a0.c1) FROM t3 AS a0 LEFT JOIN t3 AS a1 ON NULL` (1) with the same query written as a RIGHT JOIN with the tables swapped (0). Separately, the union of its HAVING partitions of a GROUP BY query gave different rows than the query itself.
