CREATE TABLE t(a INT);
SELECT count(*) FROM (SELECT * FROM range(3) r(i), (SELECT count(*) AS n FROM t) WHERE i >= n LIMIT 1);
