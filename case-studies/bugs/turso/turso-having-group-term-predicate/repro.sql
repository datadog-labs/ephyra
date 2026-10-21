CREATE TABLE t(a INTEGER PRIMARY KEY, c TEXT);
INSERT INTO t VALUES (1, 'y'), (2, 'xy'), (3, 'z');
SELECT 'q1', c FROM t GROUP BY c, a IS NULL HAVING a IS NULL;
SELECT 'q2', c FROM t GROUP BY c, a = 2 HAVING a = 2;
SELECT 'q3', c FROM t GROUP BY c, a > 1 HAVING a > 1;
SELECT 'q4', c FROM t GROUP BY c, a IN (2, 3) HAVING a IN (2, 3);
SELECT 'q5', c, a > 1 FROM t GROUP BY c, a > 1 HAVING a > 1;
