PRAGMA encoding = 'UTF-16le';
CREATE TABLE t(n, c);
INSERT INTO t VALUES (1000, CAST(printf('%.*c', 1000, 'a') AS BLOB));
INSERT INTO t VALUES (5000, CAST(printf('%.*c', 5000, 'a') AS BLOB));
SELECT n, length(c), length(CAST(c AS TEXT)) FROM t;
