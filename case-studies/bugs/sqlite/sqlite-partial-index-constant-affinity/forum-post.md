**Title:** A partial index WHERE col = <constant> makes col read back as the constant: a TEXT '5.0' comes back as '5'

```sql
CREATE TABLE t1(a, b TEXT);
INSERT INTO t1 VALUES(1, '5.0');
CREATE INDEX t1a ON t1(a) WHERE b = CAST(5 AS INT);
SELECT b FROM t1 WHERE b = CAST(5 AS INT);
SELECT count(*) FROM t1 WHERE b = CAST(5 AS INT) AND b LIKE '%.0';

CREATE TABLE t2(a INT, b INT) STRICT;
INSERT INTO t2 VALUES(1, -9223372036854775808);
CREATE INDEX t2a ON t2(a) WHERE b = -9223372036854775808.0;
SELECT b, typeof(b) FROM t2 WHERE b = -9223372036854775808.0;
```

Output from trunk (check-in dc388e83b05d, 2026-10-10 19:19), built from the source tarball with default options:

```
5
0
-9.2233720368547758e+18|real
```

Expected, which is what the same queries return with `NOT INDEXED`:

```
5.0
1
-9223372036854775808|integer
```

The WHERE clause selects the row but should not change `b`. `b = CAST(5 AS INT)` compares with NUMERIC affinity, so the text '5.0' qualifies, but it is still '5.0'. The INTEGER -9223372036854775808 equals the REAL -9223372036854775808.0, but a STRICT INT column cannot even hold that REAL: `INSERT INTO t2 VALUES(2, -9223372036854775808.0)` fails with "cannot store REAL value in INT column t2.b".

More forms of the TEXT case fail the same way: `CAST(5 AS REAL)` (a stored '5' comes back as '5.0'), `CAST('5' AS NUMERIC)` (' 5' comes back as '5'), `IS` in both the index and the query, an index on `b` itself, and a non-covering index. `INSERT INTO ... SELECT` stores the wrong value. A plain literal (`b = 5`) is fine. The -2^63 case also happens in ordinary INT and NUMERIC columns, and the other way round: a REAL -9223372036854775808.0 stored in an ordinary INT column (INT affinity leaves it a REAL) comes back as an INTEGER under an index `WHERE b = -9223372036854775808`. REAL columns are fine. No `.testctrl optimizations` setting changes the results, not even `0xffffffff`. Only `NOT INDEXED` does.

**Cause, as far as we can tell.** This comes from the 3.44.0 optimization that replaces a column with the constant implied by a partial index's WHERE clause (check-in 8d4160910d651246). `wherePartIdxExpr()` registers `b` when it has TEXT or numeric affinity. `exprPartidxExprLookup()` then codes the constant plus `OP_Affinity` with b's affinity, instead of reading `b`. That is right only if `b = <constant>` pins down the stored value, and that fails in two ways:

- The column is TEXT and the constant has a numeric affinity. The comparison then applies NUMERIC affinity to the column, so '5', '5.0' and ' 5' all qualify. Constant propagation already refuses such constants (`constInsert()`: `if( sqlite3ExprAffinity(pValue)!=0 ) return;`), and the first two queries are right without the index.
- The constant is -9223372036854775808 and the column is INT or NUMERIC. The INTEGER and the REAL compare equal, but `sqlite3VdbeIntegerAffinity()` does not convert the REAL (`ix>SMALLEST_INT64`, ticket #3922), so the column can hold either.

Two related problems are reported separately: an OR term on such a partial index can fail with "internal query planner error" ([forum post 3ede79ce42](https://sqlite.org/bugs/forumpost/3ede79ce42)), and constant propagation has the same -2^63 problem without any index.

**Versions** (official amalgamations or tarballs, default options): 3.8.2, 3.20.0, 3.28.0, 3.35.0, 3.39.0, 3.42.0 and 3.43.0 return the stored values (the STRICT part from 3.37.0, which added STRICT tables). 3.44.0, 3.45.0, 3.46.1, 3.47.0, 3.49.0, 3.50.0, 3.51.0, 3.53.4, 3.54.0 and trunk dc388e83b05d give the wrong results above.

We found it with a property-based test generator for SQLite databases and queries: it split the rows of a STRICT table by a WHERE term into the parts where the term is true, false and NULL, and the parts did not add up to the unsplit table, because the "true" part returned the INT column as a REAL.
