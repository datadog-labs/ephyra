-- Self-join elimination makes a relation laterally depend on itself
CREATE TABLE t (a int PRIMARY KEY);
SELECT s.y
FROM t AS d
LEFT JOIN (t AS x
           CROSS JOIN LATERAL (SELECT x.a AS v OFFSET 0) AS b
           JOIN LATERAL (SELECT c.a, coalesce(b.v, 0) FROM t AS c) AS s(a, y)
             ON s.a = x.a)
  ON true;
SET enable_self_join_elimination = off;
SELECT s.y
FROM t AS d
LEFT JOIN (t AS x
           CROSS JOIN LATERAL (SELECT x.a AS v OFFSET 0) AS b
           JOIN LATERAL (SELECT c.a, coalesce(b.v, 0) FROM t AS c) AS s(a, y)
             ON s.a = x.a)
  ON true;
