# Comment on tursodatabase/turso#10496

Posted as https://github.com/tursodatabase/turso/issues/10496#issuecomment-6445161010 on 2026-10-27, a comment on the existing issue #10496 (same unguarded check; this adds the foreign_keys=OFF case). The text below is the comment as posted.

---

The same check also fails while foreign keys are off (the default), where SQLite does not look at foreign keys during DML at all:

```sql
CREATE TABLE p(a, b);
CREATE TABLE c(x REFERENCES p(z));
INSERT INTO p VALUES (1, 2);
UPDATE p SET a = 5;
SELECT * FROM p;
```

Turso (`tursodb -q -m list :memory:`):
```
  × foreign key mismatch - "c" referencing "p"

1|2
```

SQLite 3.50.4 and trunk:
```
5|2
```

Every UPDATE on `p` fails, whatever it assigns; INSERT into `p`, DELETE from `p` and DML on `c` work. A key with no parent column list into a table with no PRIMARY KEY (`c(x REFERENCES p)`) does the same. The FK-cascade check in the UPDATE path (core/translate/optimizer/mod.rs, the loop over `referencing_fks` before `parent_key_may_change`) resolves every key that references the table through `Schema::resolved_fks_referencing`, which returns the mismatch error, and it is not gated on `connection.foreign_keys_enabled()`. Tested on Turso main 222c39296.
