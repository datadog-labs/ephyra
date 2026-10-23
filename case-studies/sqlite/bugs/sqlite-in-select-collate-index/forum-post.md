**Title:** x IN (SELECT y COLLATE NOCASE ...) misses rows when x has an index

```sql
CREATE TABLE t1(a);
INSERT INTO t1 VALUES('a'), ('A');
CREATE TABLE t2(b);
INSERT INTO t2 VALUES('A');
CREATE INDEX t1a ON t1(a);
SELECT group_concat(a) FROM t1 WHERE a IN (SELECT b COLLATE NOCASE FROM t2);
```

Output from trunk (check-in dabff5c1cde6, 2026-10-14 16:11), built from the source tarball with default options. 3.54.0, 3.53.4 and every release back to 3.28.0 that we tried give the same:

```
A
```

Expected `a,A`. datatype3.html section 7.1 says that "The expression "x IN (SELECT y ...)" is handled in the same way as the expression "x = y" for the purposes of determining the collating sequence", and an explicit COLLATE on either operand wins, so the comparison is NOCASE. That is the answer without the index, with `FROM t1 NOT INDEXED`, and from `SELECT a, a IN (SELECT b COLLATE NOCASE FROM t2) FROM t1` (1 for both rows). The row-value form `(a,1) IN (SELECT b COLLATE NOCASE, 1 FROM t2)` also gives a,A with the index. The COLLATE can also sit inside a function in the subquery's result column (`SELECT max(b COLLATE NOCASE) ...`), which is how we found it.

The planner also prefers the BINARY index when a NOCASE one exists: with `CREATE INDEX t1n ON t1(a COLLATE NOCASE)` added, the plan is still `SEARCH t1 USING COVERING INDEX t1a (a=?)`.

This is not the case in [forum post 0064c25b3f](https://sqlite.org/bugs/forumpost/0064c25b3f) ("OR-to-IN Conversion Loses Column Collation", fixed by check-in 984c9b1818 for 3.54.0): that was the rewrite of `x = a OR x = b` into IN in `exprAnalyzeOrTerm()`. This one is `x IN (SELECT ...)` choosing an index, and it still fails on 3.54.0 and trunk.

**Cause, as far as we can tell.** `indexInAffinityOk()` in where.c decides which collation a WO_IN term uses, so that it can be matched against the index column. For a vector left side, it pairs the field with the subquery's result column in a temporary TK_EQ. For a scalar left side, it passes the TK_IN itself to `sqlite3ExprCompareCollSeq()`, whose `pRight` is NULL, so only the left side's collation (BINARY) is considered. The BINARY index then serves a NOCASE comparison. Handling the scalar case like the vector case fixes the query above:

```diff
--- a/src/where.c
+++ b/src/where.c
@@ -334,6 +334,12 @@
     assert( ExprUseXSelect(pX) );
     inexpr.pRight = pX->x.pSelect->pEList->a[iField].pExpr;
     pX = &inexpr;
+  }else if( ExprUseXSelect(pX) ){
+    inexpr.flags = 0;
+    inexpr.op = TK_EQ;
+    inexpr.pLeft = pX->pLeft;
+    inexpr.pRight = pX->x.pSelect->pEList->a[0].pExpr;
+    pX = &inexpr;
   }
 
   if( sqlite3IndexAffinityOk(pX, idxaff) ){
```

With the patch, the NOCASE index t1n is chosen for this query when it exists, and the BINARY index alone is no longer used for it. The in*, collate*, rowvalue, rowvalue3, rowvalue4, where*, subquery* and e_expr tests we ran all pass. Take it as a pointer to the cause rather than a proposed fix.

**Versions** (official amalgamations or tarballs, default options): 3.28.0 to 3.49.0, 3.50.4, 3.53.4, 3.54.0 and trunk dabff5c1cde6 all give `A`. We did not try releases before 3.28.0.

We found it with a property-based test generator for SQLite databases and queries: the union of `WHERE p`, `WHERE NOT p` and `WHERE p IS NULL` lost two rows of the table.
