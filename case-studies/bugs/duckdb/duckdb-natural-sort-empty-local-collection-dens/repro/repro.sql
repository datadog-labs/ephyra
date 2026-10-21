SET threads = 1;
SELECT x, dense_rank() OVER ()
FROM (VALUES (7), (NULL)) t(x)
ANTI JOIN (VALUES (7), (2), (1)) s(y) ON x = y AND NULL::BOOLEAN
WHERE x IS NULL;
