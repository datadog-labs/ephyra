SELECT count(*), count(DISTINCT r.a) FROM range(1) t LEFT JOIN range(1) r(a) ON false AND r.a IN (SELECT 3) WHERE true;
