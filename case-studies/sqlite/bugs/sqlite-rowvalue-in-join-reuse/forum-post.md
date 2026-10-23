**Title:** Row-value IN in a join repeats rows: the inner loop iterates the full IN list, with duplicates, for one of its columns

```sql
CREATE TABLE t1(a PRIMARY KEY, b);
INSERT INTO t1 VALUES(1, 1);
ANALYZE;
SELECT count(*) FROM t1 x, t1 y WHERE x.a = y.a AND (x.a, x.b) IN ((1, 1), (1, 2));
SELECT x.a, x.b FROM t1 x, t1 y WHERE x.a = y.a AND (x.a, x.b) IN ((1, 1), (1, 2), (1, 3));
```

Output from trunk (check-in 69d51c35426f, 2026-10-16 20:31), built from the source tarball with default options. 3.54.0 gives the same:

```
2
1|1
1|1
1|1
```

Expected:

```
1
1|1
```

t1 has one row, (1, 1), so the self-join has one row, and `(1, 1) IN ((1, 1), (1, 2))` is true. The query returns the row once per element of the IN list whose first field is 1. The database is fine: integrity_check says ok. The same thing happens with `IN (VALUES(1, 1), (1, 2))` and with `IN (SELECT p, q FROM t2)` where t2 holds (1,1), (1,2), (1,3) (3 instead of 1), and on a WITHOUT ROWID table with the column repeated, which is how we found it:

```sql
CREATE TABLE t3(c0, PRIMARY KEY(c0)) WITHOUT ROWID;
INSERT INTO t3 VALUES('k');
ANALYZE;
SELECT count(*) FROM t3 AS a0 JOIN t3 AS a1 USING (c0) WHERE (a0.c0, a0.c0) IN (('k', NULL), ('k', 'k'));
-- 2, expected 1
```

The result is right without the ANALYZE (the plan then seeks y with x.a only), with the IN expression in the select list (`SELECT sum(((x.a, x.b) IN ((1, 1), (1, 2))) IS TRUE) FROM t1 x, t1 y WHERE x.a = y.a` gives 1), with a scalar IN (`x.a IN (1, 1)`), and when the indexed field is not the first one (`(x.b, x.a) IN ((1, 1), (2, 1))` gives 1).

**Cause, as far as we can tell.** The plan is:

```
|--SCAN x
|--LIST SUBQUERY 2
|  |--SCAN 2 CONSTANT ROWS
|  |--CREATE BLOOM FILTER
|  `--CREATE BLOOM FILTER
|--SEARCH y USING COVERING INDEX sqlite_autoindex_t1_1 (a=?)
`--REUSE LIST SUBQUERY 4
```

Through the equivalence x.a = y.a, the planner uses field 1 of the vector IN term to drive y's index (`a=?`). In the loop on x, the whole term `(x.a, x.b) IN (...)` is coded as an ordinary test, which materializes the two-column RHS in an ephemeral index (cursor 3) and sets EP_Subrtn on the IN expression. For y's loop, `codeINTerm()` calls `removeUnindexableInClauseTerms()`, which copies the expression with `sqlite3ExprDup()` (EP_Subrtn and `y.sub` included) and drops the unused field b. It clears EP_Subrtn only when a kept field is out of place (`iField!=nRhs`); here field a is first, so the flag stays and `sqlite3CodeRhsOfIN()` reuses the two-column table with OP_OpenDup:

```
29      Once           0     32    0
30      Gosub          2     7     0
31      OpenDup        4     3     0
32      Rewind         4     42    0
33        Column         4     0     9        r[9]= cursor 4 column 0
34        IsNull         9     41    0
35        SeekGE         2     41    9     1   key=r[9]
```

The loop on cursor 4 reads column 0 of every RHS row, so the value 1 comes out once for (1,1) and again for (1,2), and y is visited twice. A one-column table built for the reduced expression would hold 1 only once, since its keys are distinct. The full term is still checked on x, but it is true for that row, so it does not remove the extra copies. Clearing EP_Subrtn whenever a field was removed fixes every query above:

```diff
--- a/src/wherecode.c
+++ b/src/wherecode.c
@@ -617,6 +617,12 @@
           }
           nRhs++;
         }
+      }
+      if( nRhs<pOrigRhs->nExpr ){
+        /* Some RHS columns were removed.  Rows of the original RHS that
+        ** differ only in the removed columns become duplicates, so the
+        ** original materialization cannot be reused for the reduced IN. */
+        ExprClearProperty(pNew, EP_Subrtn);
       }
       sqlite3ExprListDelete(db, pOrigRhs);
       if( pOrigLhs ){
```

With it, y's loop builds its own one-column list (`LIST SUBQUERY 4` in place of `REUSE LIST SUBQUERY 4`) and the queries return one row. The in, in2 to in6, rowvalue* and where* tests pass as before, and veryquick shows no new failures. Take it as a pointer to the cause rather than a proposed fix.

**Versions** (official builds, default options): 3.44.0, 3.50.4, 3.51.0, 3.53.4, 3.54.0 and trunk give the wrong counts for every query above. 3.37.0 to 3.40.0 give the right answers for the IN list form, and 3.25.0 to 3.40.0 for the VALUES form, because they seek y with x.a and do not use the IN term there. The `IN (SELECT p, q FROM t2)` form is wrong from 3.35.0 (3, with `REUSE LIST SUBQUERY 1` in the plan) and right in 3.25.0 to 3.33.0, which seek y with x.a. Row-value IN lists give "row value misused" before 3.37.0.

We found it with a property-based test generator for SQLite databases and queries: `SELECT count(*) FROM t1 AS a0 JOIN t1 AS a1 USING (c0) WHERE (a0.c0, a0.c0) IN (('k68249', NULL), ('k68249', 'k68249'))` returned 2, and the same predicate summed as `... IS TRUE` in the select list over the same join returned 1.
