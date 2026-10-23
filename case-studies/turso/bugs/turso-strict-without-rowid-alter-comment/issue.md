DRAFT, not filed (comment on existing issue #9572, not a new issue)
Title: (comment on #9572) The missing comma also corrupts a SQLite-made STRICT, WITHOUT ROWID table on ALTER TABLE

The same formatting bug turns a valid table made by SQLite into a schema that neither engine can parse. Any ALTER TABLE that rewrites the table's SQL writes `... WITHOUT ROWID STRICT`:

```
sqlite3 st.db 'CREATE TABLE t1(c0 ANY, c1 ANY, PRIMARY KEY(c1)) WITHOUT ROWID, STRICT; CREATE TABLE t2(a);'
tursodb --experimental-without-rowid st.db 'ALTER TABLE t1 RENAME c0 TO r0;'     -- succeeds
tursodb --experimental-without-rowid st.db 'SELECT 1;'
-- Error: near "STRICT": syntax error
sqlite3 st.db 'SELECT 1;'
-- Error: in prepare, malformed database schema (t1) - near "STRICT": syntax error (11)
```

Tested on Turso main cbd5ac4e.
