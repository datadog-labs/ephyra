CREATE TABLE a(x int);
CREATE TABLE b(x int);
CREATE TABLE c(x int);
CREATE TABLE d(x int);
VACUUM b, d;
SET cpu_tuple_cost = 1;
SET parallel_tuple_cost = 0;
SET min_parallel_table_scan_size = 0;
SELECT 1 FROM a LEFT JOIN b ON true LEFT JOIN c ON c.x = b.x LEFT JOIN d ON d.x = c.x;
