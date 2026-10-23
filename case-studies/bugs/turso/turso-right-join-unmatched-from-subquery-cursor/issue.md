Filed as #10390 (https://github.com/tursodatabase/turso/issues/10390) on 2026-10-21.
Title: Panic "cursor id N is None" in a RIGHT JOIN's unmatched-rows pass when a WHERE subquery has a FROM subquery and a nested subquery that reads it

```sql
CREATE TABLE t1(a);
CREATE TABLE t2(a);
CREATE TABLE t3(a);
INSERT INTO t1 VALUES (1);
INSERT INTO t3 VALUES (1);
SELECT * FROM t2 RIGHT JOIN t1 ON 0
WHERE EXISTS (SELECT 1 FROM (SELECT * FROM t3) AS b WHERE b.a > 0 AND t2.a IS NULL AND 1 IN (SELECT b.a FROM t1));
```

Turso (`tursodb -q -m list :memory: < repro.sql`):
```
thread 'main' panicked at core/vdbe/mod.rs:1676:32:
cursor id 4 is None
```

SQLite 3.50.4 (trunk gives the same):
```
|1
```

Expected: one row. The t1 row has no match, so it is NULL-extended (`t2.a` is NULL), and the EXISTS is true for it.

The panic needs all of these:
- the RIGHT JOIN has an unmatched right row (`ON 0`, or an empty `t2`);
- the WHERE subquery reads the RIGHT JOIN's left table (`t2.a IS NULL`);
- the subquery reads a FROM subquery (`(SELECT * FROM t3) AS b`) with a WHERE term on it (`b.a > 0`);
- a subquery nested inside reads that FROM subquery's column (`SELECT b.a FROM t1`).

Dropping any one of them avoids it. With `ON t2.a = t1.a` and a match for every t1 row, the query works.

Likely cause: `emit_unmatched_right_rows` (core/translate/main_loop/close.rs) emits a copy of the WHERE subquery for the NULL-extended rows. In the main loop, `b` is materialized into an ephemeral index (cursor 4 above) and the nested subquery reads its column from that cursor. In the copy, `b` is planned again as a coroutine, but the nested subquery still reads `Column 4`. With `ON 0` the main-loop copy never runs, so cursor 4 was never opened. EXPLAIN shows the second copy's `InitCoroutine` and, inside it, `Column 4 0` (`ephemeral(ephemeral_subquery_t3).c0`).

#9850 is another bug in the same function: subqueries in a later join's ON clause are not emitted again for the unmatched rows at all. Here the WHERE subquery is emitted again, but its nested subquery reads the cursor of the first copy.

Tested on Turso main db441331.
