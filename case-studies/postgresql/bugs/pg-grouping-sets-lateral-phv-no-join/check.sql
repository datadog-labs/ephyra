SELECT 'q1';
CREATE TABLE gs_lat_t (x int);
INSERT INTO gs_lat_t VALUES (1), (2);
SELECT 'q2';
SELECT s.y, count(*) FROM gs_lat_t b LEFT JOIN gs_lat_t a ON a.x = b.x CROSS JOIN LATERAL (SELECT a.x AS y FROM gs_lat_t c, gs_lat_t d) s GROUP BY ROLLUP (s.y) ORDER BY 1;
SELECT 'q3';
SELECT s.y, count(*) FROM gs_lat_t b LEFT JOIN gs_lat_t a ON a.x = b.x CROSS JOIN LATERAL (SELECT a.x AS y FROM gs_lat_t c LEFT JOIN gs_lat_t d ON true) s GROUP BY CUBE (s.y) ORDER BY 1;
SELECT 'q4';
DROP TABLE gs_lat_t;
