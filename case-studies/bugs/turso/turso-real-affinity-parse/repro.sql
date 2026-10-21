CREATE TABLE r(x REAL);
INSERT INTO r VALUES ('2e218'), (2e218);
SELECT 'q1', x = 2e218, x = CAST('2e218' AS REAL) FROM r;
SELECT 'q2', count(DISTINCT x) FROM r;
SELECT 'q3', sum('2e218') = 2e218, total('2e218') = 2e218, avg('2e218') = 2e218;
SELECT 'q4', sum('1.0e-300') = CAST('1.0e-300' AS REAL);
