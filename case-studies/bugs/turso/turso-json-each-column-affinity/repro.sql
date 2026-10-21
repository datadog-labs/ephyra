SELECT count(*) FROM json_each('{"a":1}') WHERE fullkey < 0;
SELECT fullkey < 0, path < 0 FROM json_each('{"a":1}');
CREATE TABLE t AS SELECT key, value, type, fullkey FROM json_each('{"a":1}');
SELECT sql FROM sqlite_schema WHERE name = 't';
