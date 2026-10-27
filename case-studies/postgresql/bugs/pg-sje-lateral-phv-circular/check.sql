SELECT 'q1';
CREATE TABLE sje_lat_t (a int PRIMARY KEY);
INSERT INTO sje_lat_t VALUES (1), (2);
SELECT 'q2';
SELECT s.y FROM sje_lat_t AS d LEFT JOIN (sje_lat_t AS x CROSS JOIN LATERAL (SELECT x.a AS v OFFSET 0) AS b JOIN LATERAL (SELECT c.a, coalesce(b.v, 0) FROM sje_lat_t AS c) AS s(a, y) ON s.a = x.a) ON true ORDER BY 1;
SELECT 'q3';
SELECT s.y FROM sje_lat_t AS d FULL JOIN (sje_lat_t AS x CROSS JOIN LATERAL generate_series(1, x.a) AS b(v) JOIN LATERAL (SELECT c.a, coalesce(b.v, 0) FROM sje_lat_t AS c) AS s(a, y) ON s.a = x.a) ON true ORDER BY 1;
SELECT 'q4';
DROP TABLE sje_lat_t;
