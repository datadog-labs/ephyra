DRAFT, not filed
Title: (comment on #9046) sum() overflow is also not cleared by a later BLOB or non-numeric TEXT value

Comment text for #9046:

The same happens when the value after the overflow is a BLOB or a text that is not a number:

```sql
SELECT sum(x) FROM (SELECT 9223372036854775807 AS x UNION ALL SELECT 1 UNION ALL SELECT x'00');
SELECT sum(x) FROM (SELECT 9223372036854775807 AS x UNION ALL SELECT 1 UNION ALL SELECT 'abc');
```

Turso:
```
Runtime error: integer overflow
Runtime error: integer overflow
```

SQLite 3.50.4:
```
9.22337203685478e+18
9.22337203685478e+18
```

In SQLite's `sumStep`, once the sum is approximate, any value whose `sqlite3_value_numeric_type` is not INTEGER (REAL, BLOB, or TEXT that does not look like a number) sets `p->ovrfl = 0`. In Turso only a REAL value clears `ovrfl` (`Value::Numeric(Numeric::Float)` in the Sum/Total step, core/vdbe/execute.rs). `handle_text_sum` never clears it, neither for its approximate integer path (BLOB, partly numeric text) nor for its float path. A text that does look like an integer (`'1'`) is INTEGER in SQLite too, so that case correctly still raises the error.

Tested on Turso main cbd5ac4e.
