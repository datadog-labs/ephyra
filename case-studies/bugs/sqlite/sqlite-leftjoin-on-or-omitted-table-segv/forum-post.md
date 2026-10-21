**Title:** An OR in a LEFT JOIN's ON clause drives a loop on the left table: wrong rows, or a segfault when the joined table is omitted

```sql
CREATE TABLE t1(a, b);
CREATE INDEX t1b ON t1(b);
CREATE TABLE t2(c);
INSERT INTO t1 VALUES(1,1),(2,1),(3,1),(4,1),(5,1),(6,1);
ANALYZE;
SELECT DISTINCT y.a FROM t1 AS x JOIN t1 AS y ON y.b >= x.b AND y.b <= x.b
  LEFT JOIN t2 ON y.a = 5 OR y.a = 6
 WHERE y.b BETWEEN 1 AND 2 AND y.b = x.b;
SELECT DISTINCT y.a FROM t1 AS x JOIN t1 AS y ON y.b >= x.b AND y.b <= x.b
  LEFT JOIN t2 ON t2.c = y.a OR y.a = 5
 WHERE y.b BETWEEN 1 AND 2 AND y.b = x.b;
```

Output from trunk (check-in 5bd093e261a7, 2026-10-16 18:11), built from the source tarball with default options. 3.54.0 gives the same:

```
5
6
Segmentation fault (exit status 139)
```

Expected:

```
1
2
3
4
5
6
1
2
3
4
5
6
```

t2 is empty, so the LEFT JOIN cannot remove any row of y. The first query returns only the rows that satisfy the ON clause. The second query crashes. A build with `--enable-debug` stops at `assert( pC!=0 )` in OP_Column. Under AddressSanitizer it is a READ at address 0x50, which is `pC->aOffset` of a NULL VdbeCursor. The query plan is the same for both:

```
QUERY PLAN
|--MULTI-INDEX OR
|  |--INDEX 2
|  |  `--SCAN y
|  `--INDEX 3
|     `--SCAN y
|--SEARCH x USING COVERING INDEX t1b (b=?)
`--USE TEMP B-TREE FOR DISTINCT
```

The plan depends on the costs. Without the ANALYZE, with 5 rows instead of 6, or with `x.b = y.b` instead of `y.b = x.b`, y is a plain index range scan and both results are right. We found it as a crash with a fuzzed three-way self-join of one table, `... JOIN t0 AS a1 ON (a1.c1 <> a0.c1 AND a1.c1 >= a0.c1) LEFT JOIN t0 AS a2 ON ((a2.c1 = a1.c0) OR (a1.c0 IN (SELECT DISTINCT a0.c0 FROM t0 AS b0 WHERE c1 LIKE '2024-12-28')))`. The IN subquery and its correlation are not needed.

**Cause, as far as we can tell.** `whereLoopAddOr()` (where.c) accepts any OR term whose `indexable` mask includes the table. Here that term is the ON clause of the LEFT JOIN (EP_OuterON, iJoin = t2), and the table is y, which is to the left of the join. For an ordinary term, exprAnalyze adds `extraRight` to `prereqRight`, which keeps an ON term of a LEFT JOIN from driving a table to the left of the join (ticket #3015). That also keeps each OR branch from using its own term on y, so every branch is a scan of y that uses only the WHERE range on b. whereLoopAddOr takes the loop's prereq from the branches alone. The result is a MULTI-INDEX OR loop on y with no prerequisites. It has the ON term as `aLTerm[0]`, and the planner thinks it is cheaper than the plain range scan.

When t2 is kept (no DISTINCT, or a t2 column in the result), this is harmless. `pOrTab` (wherecode.c) contains the not-yet-ready t2, so the branch conditions are not coded at y. The OR term stays untested, so it is coded again at t2's level. The two branch scans are unfiltered, and the result is right. With DISTINCT, `whereOmitNoopJoin()` drops t2, marks its ON terms TERM_CODED, and decrements nLevel. y's loop still codes the OR term, and `pOrTab` no longer contains t2. In the branch sub-WHERE, t2's cursor has no mask bit, so the branch conditions look ready and are coded as filters on y. That is the first query: `y.a = 5` and `y.a = 6` filter y. A branch that reads t2 reads cursor 2, whose OpenRead was never coded because the table was omitted. That is the second query. Up to 3.50.4, exprAnalyze raised "ON clause references tables to its right" for an EP_OuterON term whose join table has no mask bit. So for these queries, 3.45.3 to 3.50.4 fail in prepare with that error instead. Since that check was removed (check-in e0beda7ffe, 2025-08-25, as far as we can tell), the queries give the wrong result or crash.

Not offering the OR optimization to an ON term of an outer join on any table but the join's own fixes both queries. That table is the only one whose OR branches can use the term:

```diff
--- a/src/where.c
+++ b/src/where.c
@@ -4862,6 +4862,8 @@
   for(pTerm=pWC->a; pTerm<pWCEnd && rc==SQLITE_OK; pTerm++){
     if( (pTerm->eOperator & WO_OR)!=0
      && (pTerm->u.pOrInfo->indexable & pNew->maskSelf)!=0
+     && (!ExprHasProperty(pTerm->pExpr, EP_OuterON)
+          || pTerm->pExpr->w.iJoin==iCur)
     ){
       WhereClause * const pOrWC = &pTerm->u.pOrInfo->wc;
       WhereTerm * const pOrWCEnd = &pOrWC->a[pOrWC->nTerm];
```

With it, y is a range scan on t1b, and both queries return 1 to 6. The fuzzed query no longer crashes (it returns no rows, as it should). The where*, join*, in*, distinct*, select* and orderby* tests (123 files) pass as before, and veryquick shows no new failures. The same change on the 3.50.4 amalgamation replaces the error with the right rows. Take it as a pointer to the cause rather than a proposed fix.

**Versions** (default options): 3.20.0, 3.25.0, 3.35.0, 3.38.0, 3.39.0, 3.40.0 and 3.44.0 return the right rows (they choose a different plan). 3.46.0, 3.50.2 and 3.50.4 fail with "ON clause references tables to its right" for both queries. 3.53.2, 3.53.4, 3.54.0 and every trunk build we have from 2026-10-07 through 5bd093e261a7 print `5`, `6` for the first query and crash on the second.

We found it with a property-based test generator for SQLite databases and queries: a DISTINCT self-join whose LEFT JOIN had an OR of an equality and an IN subquery in its ON clause crashed in sqlite3VdbeExec.
