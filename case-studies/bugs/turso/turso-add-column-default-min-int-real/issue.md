Filed as #10391 (https://github.com/tursodatabase/turso/issues/10391) on 2026-10-21.
Title: ALTER TABLE ADD COLUMN ... DEFAULT -9223372036854775808 reads as REAL for existing rows (and breaks UPDATE on a STRICT table)

```sql
CREATE TABLE t(a);
INSERT INTO t VALUES (1);
ALTER TABLE t ADD COLUMN b DEFAULT -9223372036854775808;
INSERT INTO t(a) VALUES (2);
SELECT a, b, typeof(b) FROM t;
CREATE TABLE s(a INT) STRICT;
INSERT INTO s VALUES (1);
ALTER TABLE s ADD COLUMN b INT DEFAULT -9223372036854775808;
UPDATE s SET a = 2;
SELECT a, b, typeof(b) FROM s;
```

Turso (`tursodb -q -m list :memory: < repro.sql`):
```
1|-9.22337203685478e+18|real
2|-9223372036854775808|integer
Runtime error: cannot store REAL value in INT column s.b (19) (19)
1|-9.22337203685478e+18|real
```

SQLite 3.50.4 (trunk gives the same):
```
1|-9223372036854775808|integer
2|-9223372036854775808|integer
2|-9223372036854775808|integer
```

Expected: the row that existed before the ALTER reads the default as the integer -9223372036854775808, like the row inserted after it. On a STRICT table the REAL value is written back by the UPDATE and fails the type check, so no UPDATE of an old row is possible. `DEFAULT (-9223372036854775808)` and `DEFAULT - 9223372036854775808` behave the same; `-9223372036854775807` and `9223372036854775807` are fine.

Likely cause: `eval_constant_default_value` in core/translate/alter.rs (used by the Column default in core/vdbe/builder.rs) evaluates the operand `9223372036854775808` first, which only fits a REAL, and then negates the REAL. SQLite's valueFromExpr folds a unary minus applied to a numeric literal into the literal text ("-9223372036854775808"), which converts to the integer.

Tested on Turso main db441331.
