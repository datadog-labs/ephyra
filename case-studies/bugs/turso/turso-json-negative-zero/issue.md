DRAFT, not filed

**Title:** JSON functions write a negative-zero REAL as `-0.0`; SQLite writes `0.0`

```sql
SELECT json_array(-0.0), json_object('a', -0.0), json_quote(-0.0), json(jsonb_array(-0.0));
SELECT jsonb_array(-0.0) = jsonb_array(0.0);
SELECT count(*) FROM (SELECT DISTINCT json_array(x) FROM (SELECT -0.0 AS x UNION ALL SELECT 0.0));
```

Turso (`tursodb`, main 7641fad2):

```
[-0.0]|{"a":-0.0}|-0.0|[-0.0]
0
2
```

SQLite 3.50.4 (and current trunk):

```
[0.0]|{"a":0.0}|0.0|[0.0]
1
1
```

When a SQL REAL that is negative zero becomes JSON, SQLite writes `0.0`, the same as it prints the value anywhere else (`SELECT -0.0, CAST(-0.0 AS TEXT), quote(-0.0)` gives `0.0` in both engines). Turso writes `-0.0`, so JSON built from `0.0` and `-0.0` compares unequal, and `DISTINCT` or `GROUP BY` over such JSON returns two rows where SQLite returns one. JSON *text* containing `-0.0` (`json('[-0.0]')`) is passed through unchanged by both, so only the REAL-to-JSON conversion differs.

The REAL branch of the value-to-JSONB conversion in `core/json/mod.rs` formats the float with `ryu`, which keeps the sign of zero; the non-JSON formatting already matches SQLite.

Found with a differential test generator for SQLite databases and queries, comparing Turso with SQLite 3.50.4.
