CREATE TABLE t(a);
SELECT * FROM t WHERE rowid = sqlar_uncompress(1.0, 3);
