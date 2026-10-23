CREATE TABLE t(b BOOLEAN);
INSERT INTO t VALUES (true), (false);
SELECT * FROM t WHERE contains(CASE WHEN b THEN 'a' END, 'a');
