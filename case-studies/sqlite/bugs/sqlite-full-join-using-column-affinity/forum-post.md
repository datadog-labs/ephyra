# FULL JOIN USING column reads back as REAL through a subquery that is not flattened

Posted to the SQLite bug forum as https://sqlite.org/bugs/forumpost/26213fdfb9 on 2026-10-26. The text below is the post as written.

---

```sql
CREATE TABLE t1(a REAL);
CREATE TABLE t2(a);
INSERT INTO t2 VALUES(9007199254740993);
SELECT a FROM t1 FULL JOIN t2 USING(a);
SELECT a FROM (SELECT DISTINCT a FROM t1 FULL JOIN t2 USING(a));
```

Output from trunk (check-in 75cb77338bd7, 2026-10-26 08:23), built from the source tarball:

```
9007199254740993
9007199254740992.0
```

Expected:

```
9007199254740993
9007199254740993
```

The only row comes from t2, so the USING column holds t2's integer, and the first query returns it. The second query only passes that column through a DISTINCT subquery, but returns a REAL, which here also loses precision (the integer 0 comes back as 0.0 the same way). A MATERIALIZED CTE, a DISTINCT over a view, and a subquery in a query with a window function give the same result. The plain subquery `SELECT a FROM (SELECT a FROM t1 FULL JOIN t2 USING(a))` is flattened and returns the integer, but with the query flattener disabled (`.testctrl optimizations 1`) it returns 9007199254740992.0 too.

**Versions** (release CLIs): 3.39.0, 3.40.0 and 3.50.4 return the expected output. 3.53.4, 3.54.0, 3.54.1 and trunk give the output above.

As far as we can tell, lookupName() in resolve.c now turns the USING column of a FULL JOIN into coalesce(t1.a, t2.a) with affExpr set to SQLITE_AFF_DEFER, so sqlite3ExprAffinity() gives it the affinity of t1.a, REAL. 3.50.4 does not set SQLITE_AFF_DEFER there, and the column has no affinity. The change appears to come with the tests for "Forum post 2026-05-15T11:23:01Z" in joinI.test (8.x). sqlite3SubqueryColumnTypes() in select.c gives the subquery column that REAL affinity, and the outer query reads the column with OP_RealAffinity (sqlite3ExprCodeGetColumnOfTable() in expr.c). This assumes that REAL affinity was already applied to the value, which holds for a REAL table column but not for a value that came from t2.a.

We found this with a property-based test generator for SQLite databases and queries: wrapping a FULL JOIN query in a CTE, to add a window function, changed the values it returned.
