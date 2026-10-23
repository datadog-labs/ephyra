Filed as #10398 (https://github.com/tursodatabase/turso/issues/10398) on 2026-10-21.
Title: sqlite_schema.sql holds a re-serialized statement instead of the original text, and keeps IF NOT EXISTS

```sql
CREATE TABLE t(a  INT, b VARCHAR(10) DEFAULT -5, c);
CREATE INDEX IF NOT EXISTS i ON t(a);
CREATE TRIGGER g BEFORE INSERT ON t BEGIN SELECT RAISE(IGNORE); END;
CREATE VIEW v AS SELECT a+1 AS x FROM t;
ALTER TABLE t RENAME COLUMN c TO [d];
SELECT sql FROM sqlite_schema;
SELECT name, dflt_value FROM pragma_table_info('t') WHERE dflt_value IS NOT NULL;
```

Turso (`tursodb -q -m list :memory: < repro.sql`):
```
CREATE TABLE t (a INT, b VARCHAR (10) DEFAULT - 5, [d])
CREATE INDEX IF NOT EXISTS i ON t (a)
CREATE TRIGGER g BEFORE INSERT ON t BEGIN SELECT RAISE (IGNORE); END
CREATE VIEW v AS SELECT a + 1 AS x FROM t
b|- 5
```

SQLite 3.50.4 (trunk gives the same):
```
CREATE TABLE t(a  INT, b VARCHAR(10) DEFAULT -5, "d")
CREATE INDEX i ON t(a)
CREATE TRIGGER g BEFORE INSERT ON t BEGIN SELECT RAISE(IGNORE); END
CREATE VIEW v AS SELECT a+1 AS x FROM t
b|-5
```

Expected: SQLite stores a copy of the statement as written, with only the leading keywords normalized and `TEMP` and `IF NOT EXISTS` removed (https://sqlite.org/schematab.html, section on the sql column). ALTER TABLE RENAME COLUMN replaces only the renamed tokens, quoting the new name with double quotes. `pragma_table_info().dflt_value` is the default's original text as well.

Turso instead stores the statement printed back from its AST: whitespace is changed (`t (a`, `VARCHAR (10)`, `- 5`, `a + 1`, `RAISE (IGNORE)`), `IF NOT EXISTS` is kept for indexes and triggers (tables and views drop it), and RENAME COLUMN rewrites the whole statement with `[d]`. The difference shows wherever the text is read back: `sqlite_schema`, `pragma_table_info().dflt_value`, `.schema`/`.dump`, and tools that compare the stored schema with the statement they ran to decide whether a migration has been applied. Tables in a file created by SQLite keep SQLite's text until Turso alters them.

Likely cause: the DDL paths store `stmt.to_string()` (for indexes `canonical_create_index_sql` in core/translate/index.rs) rather than the input text. COMPAT.md does not mention the difference.

Tested on Turso main db441331.
