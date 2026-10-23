SET datafusion.execution.target_partitions = 1;
SELECT count(*) OVER (PARTITION BY a + NULL) AS x,
       count(*) OVER (PARTITION BY a) AS y
FROM (SELECT 1 AS a UNION ALL SELECT 2);
