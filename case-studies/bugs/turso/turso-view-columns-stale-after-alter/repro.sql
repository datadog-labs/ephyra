CREATE TABLE t(a, b);
CREATE VIEW v AS SELECT * FROM t;
CREATE VIEW w AS SELECT 1 FROM t;
ALTER TABLE t ADD COLUMN c;
SELECT name FROM pragma_table_info('v');
SELECT name, ncol FROM pragma_table_list WHERE type = 'view' ORDER BY name;
DROP TABLE t;
SELECT name, ncol FROM pragma_table_list WHERE type = 'view' ORDER BY name;
