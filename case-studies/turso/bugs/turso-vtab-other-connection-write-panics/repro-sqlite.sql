CREATE VIRTUAL TABLE v USING memvt;
INSERT INTO v(a, b) VALUES (1, 'x');
@b SELECT rowid, a, b FROM v;
@b INSERT INTO v(a, b) VALUES (2, 'y');
@b SELECT rowid, a, b FROM v;
@c DROP TABLE v;
SELECT name FROM sqlite_schema;
