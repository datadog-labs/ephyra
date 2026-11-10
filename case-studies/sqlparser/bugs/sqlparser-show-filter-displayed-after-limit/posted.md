# `SHOW TABLES ... LIKE 'p' LIMIT n` is displayed with the `LIKE` after `LIMIT`

Posted as https://github.com/apache/datafusion-sqlparser-rs/issues/2805 on 2026-11-02. The text below is the issue as posted.

---

With dialects that take the `LIKE` filter as a suffix (ClickHouse, MySQL, Generic, ...), Display writes it after `LIMIT`, and the output reparses as a `LIKE` inside the limit expression:

```
SHOW TABLES FROM system LIKE '%user%' LIMIT 2        (ClickHouseDialect)
  -> displays as: SHOW TABLES FROM system LIMIT 2 LIKE '%user%'
  -> reparses with LIMIT (2 LIKE '%user%') and no filter
SHOW DATABASES LIKE '%de%' LIMIT 2
  -> displays as: SHOW DATABASES LIMIT 2 LIKE '%de%'   (same AST change)
SHOW TABLES LIKE 'x%' LIMIT 2                        (SnowflakeDialect, infix filter)
  -> round-trips (fine)
```

ClickHouse puts the pattern before `LIMIT`: `SHOW [FULL] [TEMPORARY] TABLES [{FROM | IN} <db>] [[NOT] LIKE | ILIKE '<pattern>'] [LIMIT <N>] ...` (https://clickhouse.com/docs/sql-reference/statements/show).

`parse_show_stmt_options` reads the suffix filter right after `show_in`, before `STARTS WITH` / `LIMIT` / `FROM`, but `Display for ShowStatementOptions` uses `"{like_in_infix}{show_in}{starts_with}{limit}{from}{like_in_suffix}"`.

Tested on main ce12f1f6 (also 0.63.0 and 0.58.0).

Found with a property-based fuzzer (round trip check); I used AI assistance for triage and verified every case above.
