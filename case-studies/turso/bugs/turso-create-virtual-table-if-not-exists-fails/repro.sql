.load target/release/libturso_ext_tests
CREATE VIRTUAL TABLE IF NOT EXISTS v USING kv_store;
SELECT name FROM sqlite_schema;
CREATE VIRTUAL TABLE w USING kv_store;
SELECT name FROM sqlite_schema;
