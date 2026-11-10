# MySQL: `FLUSH TABLES` consumes the `;`, so the next statement is rejected

Posted as https://github.com/apache/datafusion-sqlparser-rs/issues/2764 on 2026-10-29. The text below is the issue as posted.

---

`FLUSH TABLES` followed by another statement doesn't parse:

```
FLUSH TABLES; SELECT 1
  -> sql parser error: Expected: end of statement, found: SELECT at Line: 1, Column: 15
FLUSH TABLES t1, t2; SELECT 1
FLUSH TABLES WITH READ LOCK; SELECT 1
  -> same error
FLUSH PRIVILEGES; SELECT 1
  -> 2 statements (fine)
```

Each statement parses on its own. Snowflake's `ALTER SESSION` had the same problem (#1775, fixed in #1786).

Tested on main c45d1f4e (also 0.63.0 and 0.58.0).

Found with a property-based fuzzer (round trip and statement-separation checks); I used AI assistance for triage and verified every case above.
