**Title:** `x IN (SELECT …)` in HAVING converts the GROUP BY term x by the IN's affinity for the whole query (regression in 3.15.0)

```sql
CREATE TABLE t0(c0);
INSERT INTO t0 VALUES ('5');
CREATE TABLE t1(c0 INTEGER);
SELECT c0, typeof(c0), c0 = '5' FROM t0 GROUP BY c0;
SELECT c0, typeof(c0), c0 = '5' FROM t0 GROUP BY c0 HAVING c0 NOT IN (SELECT c0 FROM t1);
```

Output from the 3.53.4 CLI, built from the release amalgamation with default options:

```
5|text|1
5|integer|0
```

Expected `5|text|1` on both lines. `t0.c0` has no affinity and holds the text `'5'`. The `NOT IN` compares it with an INTEGER column, so for that comparison the INTEGER affinity is applied to `'5'`. In a GROUP BY query, though, the conversion sticks to the GROUP BY term itself. Every other reference to `c0` in the query then sees the integer 5: `typeof(c0)` is `integer`, and `c0 = '5'` is false.

The same query without GROUP BY, or with the `NOT IN` in WHERE instead of HAVING, returns `5|text|1`. Other forms also fail:

- the IN in the result columns instead of HAVING;
- a REAL value meeting an INTEGER-affinity column (`3.0|real` becomes `3|integer`);
- an aggregate as the left operand meeting a TEXT column (`count(*)` reads as text).

**Versions.** 3.14.0 returns `5|text|1` on both lines. 3.15.0 and every later release we tried (3.20.0, 3.26.0, 3.31.0, 3.35.0, 3.40.0, 3.42.0 to 3.45.0, 3.50.2, 3.53.4) return `5|integer|0`, as does the macOS system `sqlite3` (3.51.0). So it looks like a regression in 3.15.0, the release that reworked IN for row values.

We found this with a property-based test generator for SQLite databases and queries, using SQLancer-style TLP oracles: partitioning the HAVING clause gave results that differed from the unpartitioned query.
