Filed as #10354 (https://github.com/tursodatabase/turso/issues/10354) on 2026-10-21.
Title: PRAGMA index_xinfo reports the rowid entry's name as '' instead of NULL

```sql
CREATE TABLE t(a);
CREATE INDEX i ON t(a);
SELECT seqno, cid, quote(name), key FROM pragma_index_xinfo('i');
```

Turso (`tursodb -q -m list :memory: < repro.sql`):
```
0|0|'a'|1
1|-1|''|0
```

SQLite 3.50.4 (trunk gives the same):
```
0|0|'a'|1
1|-1|NULL|0
```

Expected: `name` is NULL for the trailing rowid entry (cid -1), as in SQLite (https://sqlite.org/pragma.html#pragma_index_xinfo). `PRAGMA index_xinfo(i)` prints an empty string there too, which looks the same as NULL in list mode, but `name IS NULL` is false.

Likely cause: the `IndexXinfo` arm of core/translate/pragma.rs emits `program.emit_string8(String::new(), base_reg + 2)` for the rowid row instead of a NULL.

Tested on Turso main cbd5ac4e.
