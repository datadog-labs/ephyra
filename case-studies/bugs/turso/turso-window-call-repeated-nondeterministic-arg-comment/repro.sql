CREATE TABLE t(a, b);
SELECT count(*) OVER (), lag(random()) OVER (ORDER BY a) FROM t ORDER BY 2;
