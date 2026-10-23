DRAFT, not filed
Title: UPDATE on a parent table fails with "foreign key mismatch" while foreign keys are off

```sql
CREATE TABLE p(a, b);
CREATE TABLE c(x REFERENCES p(z));
INSERT INTO p VALUES (1, 2);
UPDATE p SET a = 5;
SELECT * FROM p;
```

Turso (`tursodb -q -m list :memory: < repro.sql`):
```
  × foreign key mismatch - "c" referencing "p"

1|2
```

SQLite 3.50.4 (trunk gives the same):
```
5|2
```

Expected: with `PRAGMA foreign_keys` off (the default), SQLite does not look at foreign keys during DML, so a key that names a missing parent column (`p(z)`), or one with no parent column list when the parent has no PRIMARY KEY (`c(x REFERENCES p)`), never causes an error. Turso rejects every UPDATE on `p`. INSERT into `p`, DELETE from `p`, and all DML on `c` work. With foreign keys on, SQLite also accepts `UPDATE p SET a = 5` here, because no parent key column changes; it reports the mismatch only for statements that use the key (for example a DELETE from `p`).

Likely cause: the FK-cascade check in the UPDATE DML-safety block in core/translate/optimizer/mod.rs (the loop over `referencing_fks` before `parent_key_may_change`) calls `Schema::resolved_fks_referencing` (core/schema.rs), which resolves every key that references the table and returns `fk_mismatch_err` from `resolve_fk` when a parent column is missing. The check is not gated on `connection.foreign_keys_enabled()`, and it only looks for CASCADE / SET NULL / SET DEFAULT actions, which do nothing while foreign keys are off. This is the parent-side counterpart of #10401 (fixed by #10430). The DELETE form of this failed on db441331 and works on current main.

Tested on Turso main 0f5b651c (the UPDATE also fails on db441331).
