.load target/release/libturso_ext_tests
CREATE VIRTUAL TABLE t USING kv_store;
INSERT INTO t(key, value) VALUES ('k', 'v');
UPDATE t SET rowid = rowid + 1;
