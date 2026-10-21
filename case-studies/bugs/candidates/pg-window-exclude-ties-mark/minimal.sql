-- EXCLUDE TIES with a ROWS frame that does not contain the current row.

-- 1) internal error: ERROR XX000 cannot fetch row before WindowObject's mark position
SELECT x, first_value(x) OVER (ORDER BY x ROWS BETWEEN 2 FOLLOWING AND UNBOUNDED FOLLOWING EXCLUDE TIES)
FROM (VALUES (0),(1),(1),(1)) v(x);

-- 2) silent wrong result, head side. Row 1 (x=1): frame = rows 2..3 = {1,2}; excluding its tie
--    leaves {2}, so first_value must be 2.  PostgreSQL returns NULL.  Expected: 2, NULL, NULL.
SELECT x, first_value(x) OVER (ORDER BY x ROWS BETWEEN 1 FOLLOWING AND UNBOUNDED FOLLOWING EXCLUDE TIES)
FROM (VALUES (1),(1),(2)) v(x);

-- 3) silent wrong result, tail side. Row 3 (x=2): frame = rows 1..2 = {1,2}; excluding its tie
--    leaves {1}, so last_value must be 1.  PostgreSQL returns NULL.  Expected: NULL, 1, 1.
SELECT x, last_value(x) OVER (ORDER BY x ROWS BETWEEN UNBOUNDED PRECEDING AND 1 PRECEDING EXCLUDE TIES)
FROM (VALUES (1),(2),(2)) v(x);

-- control: the aggregate path agrees with the expected frame (count of non-excluded rows)
SELECT x, count(*) OVER (ORDER BY x ROWS BETWEEN 1 FOLLOWING AND UNBOUNDED FOLLOWING EXCLUDE TIES)
FROM (VALUES (1),(1),(2)) v(x);
