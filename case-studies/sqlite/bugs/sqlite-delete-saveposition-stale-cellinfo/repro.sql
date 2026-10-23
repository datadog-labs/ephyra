-- A one-pass DELETE on a WITHOUT ROWID table deletes through a cursor that
-- keeps its position (OP_Delete with OPFLAG_SAVEPOSITION). When the leaf
-- needs no rebalance, sqlite3BtreeDelete (btree.c) leaves that cursor in
-- CURSOR_SKIPNEXT on the neighbouring cell but keeps the CellInfo of the
-- deleted cell. A later write to the same table in the same statement saves
-- the cursor's position (saveAllCursors -> saveCursorKey), which reads the
-- deleted cell's key from freed page space and its freed overflow chain:
-- "database disk image is malformed". The database itself is fine
-- (integrity_check is ok before and after).
PRAGMA page_size = 512;
CREATE TABLE t(a PRIMARY KEY, b, c) WITHOUT ROWID;
CREATE INDEX tc ON t(c);
INSERT INTO t VALUES (1, zeroblob(40), 1), (2, zeroblob(40), 2), (3, zeroblob(40), 3),
  (4, zeroblob(40), 4), (5, zeroblob(40), 5), (6, zeroblob(40), 6), (7, zeroblob(40), 7),
  (8, zeroblob(40), 8), (9, zeroblob(1300), 9);
CREATE TABLE u(x);
CREATE TRIGGER r AFTER INSERT ON u BEGIN
  -- MULTI-INDEX OR: the rows are found through one cursor on t and deleted
  -- through another, which is left in CURSOR_SKIPNEXT after the last delete.
  DELETE FROM t WHERE c IS NULL OR a > 8;
  -- Any write to t through a third cursor saves the other cursors on t.
  INSERT INTO t VALUES (0, 0, 0);
END;
PRAGMA integrity_check;
-- ok
INSERT INTO u VALUES (1);
-- Error: database disk image is malformed       expected: no error
SELECT a, length(b) FROM t;
-- 1|40 ... 8|40, 9|1300 (rolled back)   expected 0|1, 1|40 ... 8|40
PRAGMA integrity_check;
-- ok
