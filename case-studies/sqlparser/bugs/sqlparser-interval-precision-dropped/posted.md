# PostgreSQL: `INTERVAL (p) '<value>'` loses the precision

Posted as https://github.com/apache/datafusion-sqlparser-rs/issues/2763 on 2026-10-29. The text below is the issue as posted.

---

With PostgreSqlDialect (and GenericDialect), a precision written after `INTERVAL` is parsed and then dropped:

```
SELECT INTERVAL (3) '1.23456 seconds'      -- PostgreSqlDialect
  -> displays as: SELECT INTERVAL '1.23456 seconds'
  -> AST: Interval { value: '1.23456 seconds', leading_precision: None, fractional_seconds_precision: None, .. }

SELECT INTERVAL (100) MINUTE               -- GenericDialect
  -> displays as: SELECT INTERVAL MINUTE   (the 100 is gone and MINUTE becomes the value)
```

The precision changes the result in PostgreSQL (18.6):

```
SELECT INTERVAL (3) '1.23456 seconds', INTERVAL '1.23456 seconds';
-- 00:00:01.235 | 00:00:01.23456
```

The Generic case worked in 0.58.0 (`INTERVAL (100) MINUTE` round-tripped, as it still does with MySqlDialect) and changed with #1984.

Tested on main c45d1f4e (also 0.63.0).

Found with a property-based fuzzer (round trip and statement-separation checks); I used AI assistance for triage and verified every case above.
