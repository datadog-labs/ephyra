-- Pre-triage check (sqlite-coverage/deploy/known-bugs.json): sqlite-delete-saveposition-stale-cellinfo
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql
-- Its output on a correct build (the fix applied) is the registry's "expected";
-- trunk printing exactly that means the bug was fixed upstream.
PRAGMA page_size = 512;
CREATE TABLE t(a PRIMARY KEY, b, c) WITHOUT ROWID;
CREATE INDEX tc ON t(c);
INSERT INTO t VALUES (1, zeroblob(40), 1), (2, zeroblob(40), 2), (3, zeroblob(40), 3),
  (4, zeroblob(40), 4), (5, zeroblob(40), 5), (6, zeroblob(40), 6), (7, zeroblob(40), 7),
  (8, zeroblob(40), 8), (9, zeroblob(1300), 9);
CREATE TABLE u(x);
CREATE TRIGGER r1 AFTER INSERT ON u WHEN new.x = 1 BEGIN
  DELETE FROM t WHERE c IS NULL OR a > 8;
  INSERT INTO t VALUES (0, 0, 0);
END;
CREATE TABLE w(a PRIMARY KEY, b, c) WITHOUT ROWID;
CREATE INDEX wc ON w(c);
INSERT INTO w SELECT * FROM t;
CREATE TRIGGER r2 AFTER INSERT ON u WHEN new.x = 2 BEGIN
  DELETE FROM w WHERE c IS NULL OR a > 8;
  UPDATE w SET b = 1 WHERE a = 1;
END;
SELECT 'q1';
INSERT INTO u VALUES (1);
SELECT a, length(b) FROM t WHERE a < 1 OR a > 7;
SELECT 'q2';
INSERT INTO u VALUES (2);
SELECT a, length(b) FROM w WHERE a < 2 OR a > 7;
SELECT 'q3';
PRAGMA integrity_check;
