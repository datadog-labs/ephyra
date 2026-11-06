.load target/release/libturso_ext_tests
CREATE TABLE t(x);
INSERT INTO t(rowid, x) VALUES (100, 1);
CREATE VIRTUAL TABLE v USING kv_store;
INSERT INTO v(key, value) VALUES ('k1', 'x');
SELECT changes(), total_changes(), last_insert_rowid();
UPDATE v SET value = 'y';
SELECT changes(), total_changes(), last_insert_rowid();
DELETE FROM v;
SELECT changes(), total_changes(), last_insert_rowid();
