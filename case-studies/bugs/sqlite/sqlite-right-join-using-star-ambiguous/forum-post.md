**Title:** `SELECT *` over a USING join left of a RIGHT or FULL JOIN is "ambiguous column name" when a later table has that column

```sql
CREATE TABLE t0(c0, c1);
CREATE TABLE t1(c0, c1);
INSERT INTO t0 VALUES (1, 2), (4, 5);
INSERT INTO t1 VALUES (1, 3), (6, 7);
SELECT * FROM t0 FULL JOIN t1 USING (c0) CROSS JOIN t0 AS t2;
```

Output from trunk (check-in 07095b76c1f0, 2026-10-08 13:26), built from the source tarball with default options:

```
Parse error near line 5: ambiguous column name: c0
  SELECT * FROM t0 FULL JOIN t1 USING (c0) CROSS JOIN t0 AS t2;
         ^--- error here
```

The query is not ambiguous: the USING clause names `c0` of `t0` and `t1` only, and the `*` should give `c0` (the two coalesced), `t0.c1`, `t1.c1`, `t2.c0`, `t2.c1`, six rows. DuckDB returns exactly that (`(1,2,3,1,2) (1,2,3,4,5) (4,5,NULL,1,2) (4,5,NULL,4,5) (6,NULL,7,1,2) (6,NULL,7,4,5)`), and SQLite returns it too when the `*` is written out (`SELECT coalesce(t0.c0, t1.c0), t0.c1, t1.c1, t2.* FROM t0 FULL JOIN t1 ON t0.c0 = t1.c0 CROSS JOIN t0 AS t2`).

The same error for:

- `SELECT * FROM t0 RIGHT JOIN t1 USING (c0) CROSS JOIN t0 AS t2;`
- `SELECT * FROM t0 JOIN t1 USING (c0) RIGHT JOIN t0 AS t2 ON 1;` (the USING join is inner, but `t0` is a left operand of the later RIGHT JOIN);
- NATURAL in place of USING.

It needs a later table with a column named like a USING column: with `CROSS JOIN t2` where `t2(c5)` the query works, and LEFT or inner USING joins with no RIGHT or FULL JOIN after them work.

**Cause, as far as we can tell.** For a table left of a RIGHT or FULL JOIN (flagged `JT_LTORJ`), `selectExpander` expands a column that is in a later USING clause as the bare name (`pExpr = pRight`, a TK_ID) rather than as `t0.c0`, so that name resolution turns it into the coalesced USING column. Here the `*` becomes `c0, t0.c1, t1.c1, t2.c0, t2.c1`, and name resolution finds that bare `c0` in `t0`, in `t1` (folded into a coalesce, since it is in the USING clause) and in `t2` (not in the USING clause), so it reports it as ambiguous. Written out, the same name gives the same error:

```sql
SELECT c0 FROM t0 FULL JOIN t1 USING (c0) CROSS JOIN t0 AS t2;  -- ambiguous column name: c0
```

SQLite treats that written-out `c0` as ambiguous with a LEFT JOIN too, and we don't argue with that; the problem is only that the `*` produces it. Expanding to `t0.c0` instead (removing the `inAnyUsingClause` test) makes the error go away but gives NULL for `c0` in `t1`'s unmatched row (6, 7), which is presumably why the bare name is used.

This is the code that check-in d232720b79 changed (bug 2026-10-07T02:56:24Z, "TABLE.*" for the left operand of a RIGHT JOIN USING). In 3.53.4, `SELECT t0.* FROM t0 FULL JOIN t1 USING (c0) CROSS JOIN t0 AS t2` fails with the same error; on trunk it works, because `t0.*` now expands to `t0.c0`. `t1.*` and `t2.*` work too. Only the bare `*` still takes the bare-name path.

We know RIGHT JOIN name matching is strict on purpose ("Ambiguous Reference Error for Right Join", user forum, 2022-05-23). Here, though, the USING clause itself is not ambiguous: each of its two operands has exactly one `c0`. The other `c0` belongs to a table outside the USING join, and only the `*` expansion trips over it.

**Versions** (official amalgamations or tarballs, default options): 3.39.0 (the first with RIGHT JOIN), 3.42.0, 3.45.0, 3.53.4 and trunk eaec2f7f3744 and 07095b76c1f0 all fail the same way.

We found this with a property-based test generator for SQLite databases and queries, whose USING-join queries SQLite rejected.
