CREATE TEMP TABLE tt(a);
CREATE INDEX tt_i ON tt(a);
INSERT INTO tt VALUES (1), (2);
ANALYZE temp;
ANALYZE tt;
ANALYZE tt_i;
ANALYZE temp.tt;
SELECT tbl, idx, stat FROM temp.sqlite_stat1;
