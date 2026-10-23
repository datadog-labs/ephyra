Filed as #10392 (https://github.com/tursodatabase/turso/issues/10392) on 2026-10-21.
Title: STRICT type check runs before NOT NULL, so INSERT OR IGNORE fails on a row SQLite skips

```sql
CREATE TABLE t(a INT, b REAL NOT NULL) STRICT;
INSERT OR IGNORE INTO t(a) VALUES ('x');
SELECT count(*) FROM t;
INSERT INTO t(a) VALUES ('x');
```

Turso (`tursodb -q -m list :memory: < repro.sql`):
```
Runtime error: cannot store TEXT value in INT column t.a (19) (19)
0
Runtime error: cannot store TEXT value in INT column t.a (19) (19)
```

SQLite 3.50.4 (trunk gives the same):
```
0
Runtime error near line 4: NOT NULL constraint failed: t.b (19)
```

Expected: SQLite checks NOT NULL constraints before the STRICT column types. The row breaks the NOT NULL constraint on `b`, so `OR IGNORE` skips it and the statement succeeds. Turso reports the type error first, and `OR IGNORE` does not apply to it, so the whole statement fails. Without `OR IGNORE` both engines fail, but with different errors.

Likely cause: in core/translate/insert.rs the `TypeCheck` instructions for a STRICT table are emitted before `emit_notnulls`.

Related to #10326 (OR IGNORE with a NULL in a STRICT TEXT PRIMARY KEY), which is a different case: here the type error is raised before the NOT NULL check.

Tested on Turso main db441331.
