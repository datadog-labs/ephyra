# Bugs: sqlparser

The bugs the case studies found in sqlparser. Each folder has the report as
it was filed upstream, the reproduction, and where we had one, the fix and
a test. Status as of this export; the upstream link is authoritative.

| Bug | Status | Upstream |
|---|---|---|
| [x = ANY(SELECT ...) <op> y: the operator after ) is parsed into the ANY/ALL operand](sqlparser-any-all-subquery-operand-precedence/) | open | [link](https://github.com/apache/datafusion-sqlparser-rs/issues/2802) |
| [Spark/Databricks CACHE TABLE t; is rejected (only EOF ends the optional query)](sqlparser-cache-table-rejects-semicolon/) | open | [link](https://github.com/apache/datafusion-sqlparser-rs/issues/2801) |
| [DuckDB f(a == b) is displayed as f(a = b), which reparses as a named argument](sqlparser-duckdb-double-equals-named-arg/) | open | [link](https://github.com/apache/datafusion-sqlparser-rs/issues/2808) |
| [ROLLUP/CUBE/GROUPING SETS: an element starting with ( is always read as a sublist](sqlparser-grouping-element-parenthesized/) | open | [link](https://github.com/apache/datafusion-sqlparser-rs/issues/2806) |
| [PostgreSQL: `INTERVAL (p) '<value>'` loses the precision](sqlparser-interval-precision-dropped/) | open | [link](https://github.com/apache/datafusion-sqlparser-rs/issues/2763) |
| [MySQL: `FLUSH TABLES` consumes the `;`, so the next statement is rejected](sqlparser-mysql-flush-tables-consumes-semicolon/) | open | [link](https://github.com/apache/datafusion-sqlparser-rs/issues/2764) |
| [MySQL: `REGEXP`/`RLIKE` followed by `NULL` is rejected or misparsed (regression in 0.59.0)](sqlparser-regexp-rlike-null-pattern/) | open | [link](https://github.com/apache/datafusion-sqlparser-rs/issues/2760) |
| [SHOW TABLES ... LIKE 'p' LIMIT n is displayed with the LIKE after LIMIT](sqlparser-show-filter-displayed-after-limit/) | open | [link](https://github.com/apache/datafusion-sqlparser-rs/issues/2805) |
| [`SHOW ... ILIKE '<pattern>'` is displayed without quotes](sqlparser-show-ilike-pattern-unquoted/) | open | [link](https://github.com/apache/datafusion-sqlparser-rs/issues/2759) |
| [Slice with a stride and no upper bound (arr[1: :4]) is displayed as arr[1::4], which does not parse](sqlparser-slice-stride-without-upper-bound/) | open | [link](https://github.com/apache/datafusion-sqlparser-rs/issues/2807) |
| [Snowflake: `CREATE DATABASE` and `COPY INTO` consume the `;`, so the next statement is rejected](sqlparser-snowflake-create-database-copy-into-consume-semicolon/) | open | [link](https://github.com/apache/datafusion-sqlparser-rs/issues/2765) |
| [Table version clause (AT(...), FOR SYSTEM_TIME AS OF, VERSION AS OF) is displayed after the alias; output does not parse](sqlparser-table-version-display-after-alias/) | open | [link](https://github.com/apache/datafusion-sqlparser-rs/issues/2804) |
| [._name after ) or ] fails with "Unexpected character '_'"](sqlparser-underscore-field-after-paren-bracket/) | open | [link](https://github.com/apache/datafusion-sqlparser-rs/issues/2803) |
