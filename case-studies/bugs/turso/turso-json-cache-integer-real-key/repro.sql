CREATE TABLE t(c);
INSERT INTO t VALUES (1), (1.0), (0), (0.0);
SELECT c, json_extract(c, '$'), c ->> '$', json_set(c, '$.a', 1) FROM t;
