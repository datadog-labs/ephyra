-- anti-distinct: a pair sqleq-fuzz finds no counterexample for; PostgreSQL shows they differ on the rows below.
-- minimized from s13_q0_anti-distinct-d_0
CREATE TABLE t0 (c0 int4, c1 int8);
SELECT * FROM t0 AS a0_0 JOIN t0 AS a0_1 ON (a0_1.c1 NOT IN (SELECT abs(a1_0.c1) FROM t0 AS a1_0 WHERE (age('2000-03-05 12:00:00'::timestamp, '4714-11-24 00:00:00 BC'::timestamp) >= (- (CASE WHEN ('A10'::text >= '''_'::text) THEN CAST('36 hours' AS interval) ELSE CAST('100 days 23:59:59' AS interval) END)))));
SELECT DISTINCT * FROM t0 AS a0_0 JOIN t0 AS a0_1 ON (a0_1.c1 NOT IN (SELECT abs(a1_0.c1) FROM t0 AS a1_0 WHERE (age('2000-03-05 12:00:00'::timestamp, '4714-11-24 00:00:00 BC'::timestamp) >= (- (CASE WHEN ('A10'::text >= '''_'::text) THEN CAST('36 hours' AS interval) ELSE CAST('100 days 23:59:59' AS interval) END)))));

-- witness rows (load into the schema above):
INSERT INTO t0 (c0, c1) VALUES ('2'::int4, '1'::int8);
INSERT INTO t0 (c0, c1) VALUES ('2'::int4, '1'::int8);
INSERT INTO t0 (c0, c1) VALUES (NULL::int4, '-1'::int8);

-- on the witness rows, query 1 returns 3 rows, query 2 returns 2 rows:
-- 1: [[Value(Int(2)), Value(Int(1)), Value(Null), Value(Int(-1))], [Value(Int(2)), Value(Int(1)), Value(Null), Value(Int(-1))], [Value(Null), Value(Int(-1)), Value(Null), Value(Int(-1))]]
-- 2: [[Value(Null), Value(Int(-1)), Value(Null), Value(Int(-1))], [Value(Int(2)), Value(Int(1)), Value(Null), Value(Int(-1))]]
