# Bugs: sqlparser

The bugs the case studies found in sqlparser. Each folder has the report as
it was filed upstream, the reproduction, and where we had one, the fix and
a test. Status as of this export; the upstream link is authoritative.

| Bug | Status | Upstream |
|---|---|---|
| [MySQL: `REGEXP`/`RLIKE` followed by `NULL` is rejected or misparsed (regression in 0.59.0)](sqlparser-regexp-rlike-null-pattern/) | open | [link](https://github.com/apache/datafusion-sqlparser-rs/issues/2760) |
| [`SHOW ... ILIKE '<pattern>'` is displayed without quotes](sqlparser-show-ilike-pattern-unquoted/) | open | [link](https://github.com/apache/datafusion-sqlparser-rs/issues/2759) |
