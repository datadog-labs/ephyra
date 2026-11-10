# Table version clause (`AT(...)`, `FOR SYSTEM_TIME AS OF`, `VERSION AS OF`) is displayed after the alias and doesn't reparse

Posted as https://github.com/apache/datafusion-sqlparser-rs/issues/2804 on 2026-11-02. The text below is the issue as posted.

---

The parser reads the version clause before the alias and table hints, but Display writes it after them, so the output doesn't parse:

```
Snowflake:  SELECT * FROM my_table AT(OFFSET => -60*5) AS T WHERE T.flag = 'valid'
  -> displays as: SELECT * FROM my_table AS T AT(OFFSET => -60 * 5) WHERE T.flag = 'valid'
  -> reparse: Expected: identifier, found: ( at Line: 1, Column: 31

MsSql:      SELECT * FROM t FOR SYSTEM_TIME AS OF '2014-01-01' AS x WITH (NOLOCK)
  -> displays as: SELECT * FROM t AS x WITH (NOLOCK) FOR SYSTEM_TIME AS OF '2014-01-01'
  -> reparse: Expected: one of UPDATE or SHARE, found: SYSTEM_TIME at Line: 1, Column: 40

Databricks: SELECT * FROM events VERSION AS OF 123 AS e
  -> displays as: SELECT * FROM events AS e VERSION AS OF 123
  -> reparse: Expected: end of statement, found: VERSION at Line: 1, Column: 27
```

The Snowflake input is the example from https://docs.snowflake.com/en/sql-reference/constructs/at-before. SQL Server documents `table_or_view_name [ FOR SYSTEM_TIME <system_time> ] [ [ AS ] table_alias ] ... [ WITH ( <table_hint> ... ) ]` (https://learn.microsoft.com/en-us/sql/t-sql/queries/from-transact-sql), and Databricks puts the temporal spec in `table_name`, before `table_alias` (https://docs.databricks.com/aws/en/sql/language-manual/sql-ref-syntax-qry-select-table-reference).

BigQuery documents the opposite order, `table_name [ as_alias ] [ FOR SYSTEM_TIME AS OF ... ]` (https://cloud.google.com/bigquery/docs/reference/standard-sql/query-syntax#from_clause), and with BigQueryDialect `SELECT * FROM t AS x FOR SYSTEM_TIME AS OF '2020-01-01'` is rejected (`Expected: one of UPDATE or SHARE, found: SYSTEM_TIME`), while the other order parses and displays as the rejected form. So the right position seems to be dialect dependent.

Parser: `maybe_parse_table_version` runs before `maybe_parse_table_alias` in `parse_table_factor`. Display: the `TableFactor::Table` arm writes alias, index hints, `WITH (...)`, then the version. The open PR #2486 mentions this in passing and leaves it for a separate change.

Tested on main ce12f1f6 (also 0.63.0; on 0.58.0 the Databricks input is a parse error).

Found with a property-based fuzzer (round trip check); I used AI assistance for triage and verified every case above.
