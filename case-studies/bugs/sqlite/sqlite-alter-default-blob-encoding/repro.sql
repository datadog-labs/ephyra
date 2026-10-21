PRAGMA encoding = 'UTF-16le';
CREATE TABLE t1(a);
INSERT INTO t1 VALUES(1);
ALTER TABLE t1 ADD COLUMN b DEFAULT x'9e2e';   -- U+2E9E in UTF-16le, valid
CREATE TABLE t2(a, b);
INSERT INTO t2 VALUES(1, x'9e2e');             -- the same blob, stored
SELECT unicode(CAST(b AS TEXT)), length(CAST(b AS TEXT)) FROM t1;  -- 158|2, expected 11934|1
SELECT unicode(CAST(b AS TEXT)), length(CAST(b AS TEXT)) FROM t2;  -- 11934|1
SELECT unicode(CAST(x'9e2e' AS TEXT));                             -- 11934
SELECT unicode(min(CAST(b AS TEXT))) FROM t1;                      -- 158
SELECT unicode(min(CAST(b AS TEXT))) FROM t1 GROUP BY a;           -- 11934 (value went through the sorter)
SELECT CAST(b AS TEXT) = CAST(x'9e2e' AS TEXT) FROM t1;            -- 0, expected 1
