DRAFT, not filed
Title: Date/time functions don't stop at an embedded NUL in their text arguments; SQLite reads up to the NUL

SQLite's date and time functions read their arguments as C strings, so text (or a BLOB read as text) is cut at the first NUL. Turso parses the whole value, so a time value, a modifier or a format followed by a NUL and more bytes gives NULL (or keeps the bytes after the NUL). LIKE, GLOB, substr() and length() already stop at the NUL in Turso (#6305).

```sql
SELECT date('2024-01-01' || char(0) || 'x');
SELECT time(x'3700');
SELECT julianday('2024-01-01' || char(0));
SELECT date('2024-01-01', '+1 day' || char(0) || 'x');
SELECT hex(strftime('%Y' || char(0) || '%m', '2024-05-01'));
```

Turso (`tursodb -q -m list`):

```




32303234003035
```

(the first four lines are empty: NULL)

SQLite 3.50.4 (trunk is the same):

```
2024-01-01
12:00:00
2460310.5
2024-01-02
32303234
```

Expected: the SQLite output. The difference changes WHERE results, e.g. `WHERE time(c)` on a column holding the blob `x'3700'` keeps the row in SQLite and drops it in Turso.

Likely cause: the datetime functions (core/functions/datetime.rs) parse the full text value. LIKE/GLOB and friends first cut it with `sqlite_text_prefix` (core/vdbe/value.rs), which the date functions don't do.

Tested on Turso main f8549ca2.
