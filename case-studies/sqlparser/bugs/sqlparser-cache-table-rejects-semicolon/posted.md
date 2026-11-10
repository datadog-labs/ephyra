# Spark/Databricks: `CACHE TABLE t;` is rejected (only EOF ends the optional query)

Posted as https://github.com/apache/datafusion-sqlparser-rs/issues/2801 on 2026-11-02. The text below is the issue as posted.

---

`CACHE TABLE <name>` without a query only parses at the end of input. A `;` after it is an error:

```
CACHE TABLE t;
  -> sql parser error: Expected: a QUERY statement, found: ; at Line: 1, Column: 14
CACHE TABLE t; SELECT 1
CACHE LAZY TABLE t OPTIONS ('storageLevel' = 'DISK_ONLY'); SELECT 1
  -> same error
CACHE TABLE t AS SELECT 1; SELECT 1
  -> 2 statements (fine)
```

`parse_cache_table` checks `peek_token_ref().token == Token::EOF` before calling `parse_as_query`, so `;` goes to the query parser. `UNCACHE` had the same problem (#1244, fixed in #1320).

Grammar (`[ [ AS ] query ]` is optional): https://spark.apache.org/docs/latest/sql-ref-syntax-aux-cache-cache-table.html, https://docs.databricks.com/aws/en/sql/language-manual/sql-ref-syntax-aux-cache-cache-table

Tested with SparkSqlDialect and DatabricksDialect on main ce12f1f6 (also 0.63.0 and 0.58.0).

Found with a property-based fuzzer (round trip and statement-separation checks); I used AI assistance for triage and verified every case above.
