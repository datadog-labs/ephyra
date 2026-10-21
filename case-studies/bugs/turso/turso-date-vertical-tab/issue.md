DRAFT, not filed
Title: Date/time functions don't accept a trailing vertical tab (0x0B) after the time value (like #8454, for dates)

SQLite skips whitespace after a time value, and its whitespace includes the vertical tab (`char(11)`). Turso returns NULL. Other whitespace (`char(12)`, space, tab) already works.

```sql
SELECT quote(date('2024-01-01' || char(11)));
SELECT quote(datetime('12:00' || char(11)));
SELECT quote(date('2460000.5' || char(11)));
SELECT quote(datetime('0' || char(11)));
SELECT quote(date('2024-01-01' || char(12)));
```

Turso (`tursodb -q -m list`):

```
NULL
NULL
NULL
NULL
'2024-01-01'
```

SQLite 3.50.4 (trunk is the same):

```
'2024-01-01'
'2000-01-01 12:00:00'
'2023-02-25'
'-4713-11-24 12:00:00'
'2024-01-01'
```

Expected: the SQLite output. #8454 fixed the same thing for CAST and column affinity.

Likely cause: core/functions/datetime.rs skips whitespace with `char::is_ascii_whitespace` (`check_trailing_garbage` ~449, `parse_timezone` ~384, the number path in `parse_date_or_time` ~221). Rust's ASCII whitespace doesn't include 0x0B; `sqlite3Isspace` does.

Tested on Turso main abf885b2 (also on f8549ca2).
