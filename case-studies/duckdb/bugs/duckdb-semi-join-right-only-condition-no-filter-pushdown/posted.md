# Posted: https://github.com/duckdb/duckdb/issues/27633

Filed by alpaylan, 2026-10-21. Text as posted:

**SEMI JOIN with a right-side-only condition returns no rows when filter_pushdown is disabled**

### What happens?

## SQL

```sql
CREATE TABLE t1(c0 INTEGER); INSERT INTO t1 VALUES (1);
CREATE TABLE t2(c3 BOOLEAN); INSERT INTO t2 VALUES (NULL), (true);
SELECT 'default' AS mode, count(*) AS n FROM (SELECT 2 AS a0 FROM t1 AS a JOIN t1 AS b ON a.c0 = b.c0 SEMI JOIN t2 ON t2.c3);
SET disabled_optimizers='filter_pushdown';
SELECT 'no filter_pushdown' AS mode, count(*) AS n FROM (SELECT 2 AS a0 FROM t1 AS a JOIN t1 AS b ON a.c0 = b.c0 SEMI JOIN t2 ON t2.c3);
```

## Observed (2026-10-21, `-csv` output of the two count queries)

**main 5d12e1ac48 (v2.1.0-dev89736), `make relassert`**, and the same on 8f17a62d (the #27498 merge) and on
213f5993 (its parent); the workspace's main d6016290 also reproduces it:
```
default,1
no filter_pushdown,0
```

**v1.5.6 release CLI**:
```
default,1
no filter_pushdown,1
```

The correct result here is one row: `t1` has one row, the self-join keeps it, and the SEMI JOIN keeps it because `t2` has a
row where `c3` is true.


### To Reproduce

```sql
CREATE TABLE t1(c0 INTEGER); INSERT INTO t1 VALUES (1);
CREATE TABLE t2(c3 BOOLEAN); INSERT INTO t2 VALUES (NULL), (true);
SELECT 'default' AS mode, count(*) AS n FROM (SELECT 2 AS a0 FROM t1 AS a JOIN t1 AS b ON a.c0 = b.c0 SEMI JOIN t2 ON t2.c3);
SET disabled_optimizers='filter_pushdown';
SELECT 'no filter_pushdown' AS mode, count(*) AS n FROM (SELECT 2 AS a0 FROM t1 AS a JOIN t1 AS b ON a.c0 = b.c0 SEMI JOIN t2 ON t2.c3);
```

### OS:

macOS 26 (aarch64)

### DuckDB Version:

main 5d12e1ac48 (v2.1.0-dev89736), `make relassert`

### DuckDB Client:

CLI

### Hardware:

_No response_

### Full Name:

Alperen Keles

### Affiliation:

Datadoc Inc.

### Did you include all relevant configuration (e.g., CPU architecture, Linux distribution) to reproduce the issue?

- [x] Yes, I have

### Did you include all code required to reproduce the issue?

- [x] Yes, I have

### Did you include all relevant data sets for reproducing the issue?

Yes
