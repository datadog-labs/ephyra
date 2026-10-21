DRAFT, not filed (comment on existing issue #9051, not a new issue)
Title: (comment on #9051) Row-value BETWEEN also fails in a join's ON clause

The same internal error also comes up when the row-value BETWEEN is in a join's ON clause, not only in the SELECT list:

```sql
CREATE TABLE t(a, b);
INSERT INTO t VALUES (1, 2);
SELECT * FROM t WHERE (a, b) BETWEEN (0, 0) AND (5, 5);
SELECT * FROM t AS x JOIN t AS y ON x.a = y.a AND (x.a, x.b) BETWEEN (0, 0) AND (5, 5);
SELECT * FROM t AS x LEFT JOIN t AS y ON (y.a, y.b) BETWEEN (0, 0) AND (5, 5);
```

Turso:
```
1|2
Parse error: insufficient registers allocated for expression vector write (start=11, count=2, next_free=12)
Parse error: insufficient registers allocated for expression vector write (start=7, count=2, next_free=8)
```
SQLite 3.50.4:
```
1|2
1|2|1|2
1|2|1|2
```

Tested on Turso main cbd5ac4e.
