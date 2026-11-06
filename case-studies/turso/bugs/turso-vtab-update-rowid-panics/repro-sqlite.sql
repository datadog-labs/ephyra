CREATE VIRTUAL TABLE t USING memvt;
INSERT INTO t(a, b) VALUES ('k', 'v');
UPDATE t SET rowid = rowid + 1;
SELECT rowid, a, b FROM t;
