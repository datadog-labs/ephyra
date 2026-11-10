# Snowflake: `CREATE DATABASE` and `COPY INTO` consume the `;`, so the next statement is rejected

Posted as https://github.com/apache/datafusion-sqlparser-rs/issues/2765 on 2026-10-29. The text below is the issue as posted.

---

With SnowflakeDialect, these fail as soon as another statement follows:

```
CREATE DATABASE my_db; SELECT 1
  -> sql parser error: Expected: end of statement, found: SELECT at Line: 1, Column: 24
CREATE DATABASE my_db COMMENT = 'x'; SELECT 1
COPY INTO t FROM @stage; SELECT 1
  -> same error (on 0.63.0, COPY INTO gives "Expected: =, found: 1 at Line: 1, Column: 33" instead)
CREATE SCHEMA my_s; SELECT 1
  -> 2 statements (fine)
```

Each statement parses on its own. `CREATE DATABASE my_db; SELECT 1` worked in 0.58.0 and broke in 0.59.0 (#1939); `COPY INTO` has done this since #1669, which fixed #1519 for a trailing `;` only. Same class as #1775 (fixed in #1786). Open PR #2438 makes `->>` end these statements but leaves the `;` case as is.

Tested on main c45d1f4e (also 0.63.0).

Found with a property-based fuzzer (round trip and statement-separation checks); I used AI assistance for triage and verified every case above.
