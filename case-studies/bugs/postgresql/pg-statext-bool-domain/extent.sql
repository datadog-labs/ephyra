-- extent.sql: pg-statext-bool-domain. Each case prints its id first.
\set QUIET on
\pset footer off
CREATE DOMAIN db AS bool;
CREATE DOMAIN dbb AS db;               -- domain over domain over bool
CREATE DOMAIN di AS int4;
CREATE DOMAIN dt AS text;
CREATE DOMAIN dbc AS bool CHECK (VALUE IS NOT NULL);

-- A: clause forms on the base case (mcv on int8, domain-over-bool)
CREATE TABLE t(a int8, b db);
INSERT INTO t VALUES (1, true);
CREATE STATISTICS t_s (mcv) ON a, b FROM t;
ANALYZE t;
\echo A1 a = 1 AND b
SELECT * FROM t WHERE a = 1 AND b;
\echo A2 a = 1 AND NOT b
SELECT * FROM t WHERE a = 1 AND NOT b;
\echo A3 a = 1 AND b = true
SELECT * FROM t WHERE a = 1 AND b = true;
\echo A4 a = 1 AND b = false
SELECT * FROM t WHERE a = 1 AND b = false;
\echo A5 a = 1 AND b IS TRUE
SELECT * FROM t WHERE a = 1 AND b IS TRUE;
\echo A6 a = 1 AND b IS NOT FALSE
SELECT * FROM t WHERE a = 1 AND b IS NOT FALSE;
\echo A7 a = 1 AND b IS NULL
SELECT * FROM t WHERE a = 1 AND b IS NULL;
\echo A8 a = 1 OR b
SELECT * FROM t WHERE a = 1 OR b;
\echo A9 b alone
SELECT * FROM t WHERE b;
\echo A10 b AND a = 1 (order swapped)
SELECT * FROM t WHERE b AND a = 1;
\echo A11 a = 1 AND b::bool  (explicit cast)
SELECT * FROM t WHERE a = 1 AND b::bool;
\echo A12 a = 1 AND b <> false
SELECT * FROM t WHERE a = 1 AND b <> false;
\echo A13 a = 1 AND (b OR b)
SELECT * FROM t WHERE a = 1 AND (b OR b);
\echo A14 a = 1 AND b = (SELECT true)  (non-const comparand)
SELECT * FROM t WHERE a = 1 AND b = (SELECT true);
\echo A15 EXPLAIN a = 1 AND b
EXPLAIN (COSTS OFF) SELECT * FROM t WHERE a = 1 AND b;
\echo A16 DELETE ... WHERE a = 1 AND b
BEGIN; DELETE FROM t WHERE a = 1 AND b; ROLLBACK;
\echo A17 empty table, no ANALYZE data rows (stats object built on 0 rows)
CREATE TABLE t0(a int8, b db);
CREATE STATISTICS t0_s (mcv) ON a, b FROM t0;
ANALYZE t0;
SELECT * FROM t0 WHERE a = 1 AND b;
\echo A18 stats created but never analyzed
CREATE TABLE tn(a int8, b db);
INSERT INTO tn VALUES (1, true);
CREATE STATISTICS tn_s (mcv) ON a, b FROM tn;
SELECT * FROM tn WHERE a = 1 AND b;

-- B: control cases
\echo B1 plain bool column (no domain)
CREATE TABLE p(a int8, b bool);
INSERT INTO p VALUES (1, true);
CREATE STATISTICS p_s (mcv) ON a, b FROM p;
ANALYZE p;
SELECT * FROM p WHERE a = 1 AND b;
SELECT * FROM p WHERE a = 1 AND NOT b;
\echo B2 domain over int4, a = 1 AND c = 1
CREATE TABLE i(a int8, c di);
INSERT INTO i VALUES (1, 1);
CREATE STATISTICS i_s (mcv) ON a, c FROM i;
ANALYZE i;
SELECT * FROM i WHERE a = 1 AND c = 1;
SELECT * FROM i WHERE a = 1 AND c IS NULL;
SELECT * FROM i WHERE a = 1 AND c IN (1, 2);
\echo B3 domain over text, a = 1 AND c = 'x'
CREATE TABLE tx(a int8, c dt);
INSERT INTO tx VALUES (1, 'x');
CREATE STATISTICS tx_s (mcv) ON a, c FROM tx;
ANALYZE tx;
SELECT * FROM tx WHERE a = 1 AND c = 'x';
SELECT * FROM tx WHERE a = 1 AND c LIKE 'x%';
\echo B4 domain over int4 with a boolean expression on it: a = 1 AND (c > 0)
SELECT * FROM i WHERE a = 1 AND (c > 0);

-- C: other domain variants over bool
\echo C1 domain over domain over bool
CREATE TABLE t2(a int8, b dbb);
INSERT INTO t2 VALUES (1, true);
CREATE STATISTICS t2_s (mcv) ON a, b FROM t2;
ANALYZE t2;
SELECT * FROM t2 WHERE a = 1 AND b;
\echo C2 domain over bool with CHECK constraint
CREATE TABLE t3(a int8, b dbc);
INSERT INTO t3 VALUES (1, true);
CREATE STATISTICS t3_s (mcv) ON a, b FROM t3;
ANALYZE t3;
SELECT * FROM t3 WHERE a = 1 AND b;
\echo C3 two domain-bool columns, no int column
CREATE TABLE t4(b1 db, b2 db);
INSERT INTO t4 VALUES (true, true);
CREATE STATISTICS t4_s (mcv) ON b1, b2 FROM t4;
ANALYZE t4;
SELECT * FROM t4 WHERE b1 AND b2;
\echo C4 plain bool + domain bool
CREATE TABLE t5(p bool, b db);
INSERT INTO t5 VALUES (true, true);
CREATE STATISTICS t5_s (mcv) ON p, b FROM t5;
ANALYZE t5;
SELECT * FROM t5 WHERE p AND b;
\echo C5 int4 instead of int8 for a
CREATE TABLE t6(a int4, b db);
INSERT INTO t6 VALUES (1, true);
CREATE STATISTICS t6_s (mcv) ON a, b FROM t6;
ANALYZE t6;
SELECT * FROM t6 WHERE a = 1 AND b;

-- D: other statistics kinds on the same columns
\echo D1 ndistinct only
CREATE TABLE d1(a int8, b db);
INSERT INTO d1 VALUES (1, true);
CREATE STATISTICS d1_s (ndistinct) ON a, b FROM d1;
ANALYZE d1;
SELECT * FROM d1 WHERE a = 1 AND b;
SELECT a, b, count(*) FROM d1 GROUP BY a, b;
\echo D2 dependencies only
CREATE TABLE d2(a int8, b db);
INSERT INTO d2 VALUES (1, true);
CREATE STATISTICS d2_s (dependencies) ON a, b FROM d2;
ANALYZE d2;
SELECT * FROM d2 WHERE a = 1 AND b;
SELECT * FROM d2 WHERE a = 1 AND b = true;
\echo D3 default kinds (no list = ndistinct, dependencies, mcv)
CREATE TABLE d3(a int8, b db);
INSERT INTO d3 VALUES (1, true);
CREATE STATISTICS d3_s ON a, b FROM d3;
ANALYZE d3;
SELECT * FROM d3 WHERE a = 1 AND b;
\echo D4 expression statistics on (b::bool) and a, mcv
CREATE TABLE d4(a int8, b db);
INSERT INTO d4 VALUES (1, true);
CREATE STATISTICS d4_s (mcv) ON a, (b::bool) FROM d4;
ANALYZE d4;
SELECT * FROM d4 WHERE a = 1 AND b;
\echo D5 single-expression statistics on (b::bool)
CREATE TABLE d5(a int8, b db);
INSERT INTO d5 VALUES (1, true);
CREATE STATISTICS d5_s ON (b::bool) FROM d5;
ANALYZE d5;
SELECT * FROM d5 WHERE a = 1 AND b;
\echo D6 mcv on (a, b, (a+1)) mixing columns and an expression
CREATE TABLE d6(a int8, b db);
INSERT INTO d6 VALUES (1, true);
CREATE STATISTICS d6_s (mcv) ON a, b, (a + 1) FROM d6;
ANALYZE d6;
SELECT * FROM d6 WHERE a = 1 AND b;

\echo D7 mcv on (a, b, (b::bool)): the stats object also has the relabeled column as an expression
CREATE TABLE d7(a int8, b db);
INSERT INTO d7 VALUES (1, true);
CREATE STATISTICS d7_s (mcv) ON a, b, (b::bool) FROM d7;
ANALYZE d7;
SELECT * FROM d7 WHERE a = 1 AND b;
SELECT * FROM d7 WHERE a = 1 AND NOT b;

-- E: more data, realistic table
\echo E1 1000 rows, ANALYZE, a = 1 AND b
CREATE TABLE e(a int8, b db);
INSERT INTO e SELECT g % 10, g % 2 = 0 FROM generate_series(1, 1000) g;
CREATE STATISTICS e_s (mcv) ON a, b FROM e;
ANALYZE e;
SELECT count(*) FROM e WHERE a = 1 AND b;
SELECT count(*) FROM e WHERE a = 2 AND b;
\echo E2 join: e JOIN p ON e.a = p.a WHERE e.b AND e.a = 2
SELECT count(*) FROM e JOIN p ON e.a = p.a WHERE e.b AND e.a = 2;
\echo E3 view over e
CREATE VIEW ev AS SELECT * FROM e WHERE b;
SELECT count(*) FROM ev WHERE a = 2;
