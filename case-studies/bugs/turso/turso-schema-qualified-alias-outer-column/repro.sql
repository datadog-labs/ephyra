CREATE TABLE t(a);
INSERT INTO t VALUES (5);
SELECT main.x.a FROM t AS x;
SELECT (SELECT main.x.a) FROM t AS x;
SELECT * FROM t AS x WHERE EXISTS (SELECT 1 WHERE main.x.a = 5);
SELECT (SELECT count(*) FROM json_each(main.x.a)) FROM t AS x;
SELECT (SELECT main.t.a) FROM t;
