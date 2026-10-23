Filed as #9888 (https://github.com/tursodatabase/turso/issues/9888) (line 1 corrected 2026-10-21).

# strftime(): `%J` and a REAL format argument are printed with more digits than SQLite

## Reproduction

```sql
SELECT strftime('%J', 10.609676180555555);
SELECT strftime(2460324.8751736111, 0), CAST(2460324.8751736111 AS TEXT);
```

Turso (`tursodb -q -m list`, main f8549ca2):
```
10.609676180555555
2460324.875173611|2460324.87517361
```

SQLite 3.50.4:
```
10.60967618055555
2460324.87517361|2460324.87517361
```

SQLite trunk f69fd5f5 gives the same `%J` result as 3.50.4, and prints the REAL as `2460324.8751736111` in both columns (trunk changed REAL-to-text conversion).

## Expected

`%J` is `%.16g` of the julian day in SQLite (`sqlite3_str_appendf(pRes,"%.16g",x.iJD/86400000.0)`). A non-text format argument is converted to text the same way as everywhere else (`sqlite3_value_text`), so it should equal `CAST(... AS TEXT)`, which Turso gets right.

## Likely cause

In `exec_strftime` (`core/functions/datetime.rs`): `%J` prints the f64 with Rust's `{}` (shortest round-trip) outside the range [1e6, 1e7), and the format argument uses `val.to_string()` instead of the SQL text conversion.

Minor; found because a generator used a column as the format string.

Tested on Turso main f8549ca2.
