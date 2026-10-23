Filed as #10393 (https://github.com/tursodatabase/turso/issues/10393) on 2026-10-21.
Title: UPSERT DO UPDATE is compiled even when the INSERT cannot conflict; errors in it fail the INSERT, SQLite skips them

```sql
CREATE TABLE t(a, b INTEGER PRIMARY KEY);
INSERT INTO t(a) VALUES (1) ON CONFLICT DO UPDATE SET nosuch = 1;
CREATE TABLE u(a);
CREATE TRIGGER w BEFORE UPDATE ON u BEGIN INSERT INTO gone VALUES (1); END;
INSERT INTO u VALUES (1) ON CONFLICT DO UPDATE SET a = 2;
SELECT * FROM t;
SELECT * FROM u;
```

Turso (`tursodb -q -m list :memory: < repro.sql`):
```
  × Parse error: no such column: nosuch
  × Parse error: no such table: gone
```

(both SELECTs return no rows)

SQLite 3.50.4 (trunk gives the same):
```
1|1
1
```

Expected: SQLite generates the DO UPDATE branch only for a uniqueness constraint that the INSERT can violate: a UNIQUE or PRIMARY KEY index, or the rowid when the INSERT supplies it. Table `t` has only an INTEGER PRIMARY KEY that the INSERT does not set, and `u` has no constraint, so SQLite never compiles the SET clause or the UPDATE triggers, and the rows are inserted. Turso always compiles the DO UPDATE branch. So a SET column it cannot resolve, or an UPDATE trigger whose body names a table that no longer exists, fails the INSERT. When the INSERT does set the rowid (`INSERT INTO t(a, b) VALUES (1, 1) ...`) or the table has a UNIQUE index, SQLite fails too ("no such column: nosuch").

Likely cause: `resolve_upserts` in core/translate/insert.rs runs `collect_set_clauses_for_upsert` / `emit_upsert` for every upsert action without first checking that some constraint can raise a conflict.

Related to #9664, the other direction: there Turso accepts an unknown name in DO UPDATE that SQLite rejects; here Turso rejects a DO UPDATE that SQLite never compiles.

Tested on Turso main db441331.
