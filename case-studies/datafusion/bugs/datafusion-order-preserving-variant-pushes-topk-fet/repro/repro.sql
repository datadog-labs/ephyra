SET datafusion.execution.target_partitions = 4;
SELECT count(*) OVER () AS n,
       row_number() OVER (PARTITION BY length('x') ORDER BY a) AS r
FROM (SELECT 1 AS a UNION ALL SELECT 2 UNION ALL SELECT 3)
ORDER BY r LIMIT 1;
