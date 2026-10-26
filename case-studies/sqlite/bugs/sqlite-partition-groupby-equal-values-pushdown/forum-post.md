# HAVING-to-WHERE and window push-down drop rows when a group holds both 1 and 1.0

Posted to the SQLite bug forum as https://sqlite.org/bugs/forumpost/0d4c9e9e99 on 2026-10-26. The text below is the post as written.

---

```sql
CREATE TABLE t(a, b);
INSERT INTO t VALUES (1, 10), (1.0, 20);
SELECT a, sum(b) OVER (PARTITION BY a) FROM t;
SELECT * FROM (SELECT a, sum(b) OVER (PARTITION BY a) AS s FROM t)
 WHERE typeof(a) = 'integer';
SELECT a, sum(b) FROM t GROUP BY a;
SELECT a, sum(b) FROM t GROUP BY a HAVING typeof(a) = 'integer';
```

Output from trunk (check-in 75cb77338bd7, 2026-10-26 08:23), built from the source tarball. 3.54.0 gives the same:

```
1|30
1.0|30
1|10
1|30
1|10
```

Expected:

```
1|30
1.0|30
1|30
1|30
1|30
```

The integer 1 and the real 1.0 compare equal, so both rows are in one partition and in one group, as the first and third queries show. The second query filters the rows of the subquery, and the row with a = 1 has s = 30, but it comes back with s = 10. In the fourth query the HAVING clause is not an aggregate, so it is evaluated against an arbitrarily selected row of the group: the result is either the group with sum 30 or no row. Instead a sum of 10 is returned, which is the sum of no group. With `HAVING typeof(a) = 'real'` the result is `1.0` with a sum of 20.

The WHERE term is pushed into the subquery by `pushDownWindowCheck()` and the HAVING term is moved to WHERE by `havingToWhereExprCb()`, both in select.c. Both ask `sqlite3ExprIsConstantOrGroupBy()` in expr.c, which accepts any copy of a PARTITION BY or GROUP BY term that uses the BINARY collation, on the grounds that "the value of the HAVING clause term must be the same for all members of a "group"". Equal values are not always the same value: 1 and 1.0 are equal, but `typeof()`, `CAST(a AS TEXT)`, `a/2` and most other functions and operators can tell them apart. The moved term then removes single rows from the partition or group before the window function or the aggregate sees them.

The same happens when the column has a declared type but the values come from somewhere that does not apply it, for example a `UNION ALL` of an INTEGER column and a REAL column in a subquery, a view column `CAST(a AS NUMERIC)` (which leaves 1.0 as a real), a STRICT table column of type ANY, or an R-Tree auxiliary column. A comparison can also tell them apart when it applies TEXT affinity to the term:

```sql
SELECT a+0, sum(b) FROM t GROUP BY a+0 HAVING a+0 = CAST('1' AS TEXT);
```

returns `1|10`. Plain comparisons such as `HAVING a = 1` or `WHERE a IN (1, 2)` are not affected, because an integer and a real are compared by value.
**Versions:** the HAVING query returns `1|10` in every release we tried, 3.20.0 through 3.54.0. The window query returns `1|30` in 3.25.0 to 3.33.0 and `1|10` in 3.35.0 and later.

We found this with a property-based test generator for SQLite databases and queries, using SQLancer-style TLP oracles: partitioning the WHERE clause on a window subquery over an R-Tree auxiliary column gave results that differed from the unpartitioned query.
