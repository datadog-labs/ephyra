CREATE TABLE t(id INTEGER PRIMARY KEY, a, b);
INSERT INTO t VALUES (1, 5, 5), (2, NULL, 8), (3, 1, 2), (4, NULL, 2);
CREATE TABLE u(a, b);
INSERT INTO u VALUES (7, 8), (1, 2);
SELECT id, quote((a, b) NOT IN (SELECT a, b FROM u)) FROM t;
SELECT id FROM t WHERE (a, b) NOT IN (SELECT a, b FROM u);
