DRAFT, not filed

# timediff() gives wrong (sometimes malformed) results for dates with an overflowing day such as 2023-02-29

## Reproduction

```sql
SELECT timediff('2023-02-29', '2023-03-01');
SELECT timediff('2024-03-15', '2023-02-29');
SELECT timediff('2000-02-31', '2000-01-01');
SELECT date('2023-02-29'), date('2000-02-31');
```

Turso (`tursodb -q -m list`, main f8549ca2):
```
+-001-11-28 00:00:00.000
+0001-00-15 00:00:00.000
+0000-01-30 00:00:00.000
2023-03-01|2000-03-02
```

SQLite 3.50.4 (trunk f69fd5f5 gives the same):
```
+0000-00-00 00:00:00.000
+0001-00-14 00:00:00.000
+0000-02-01 00:00:00.000
2023-03-01|2000-03-02
```

## Expected

The SQLite results. Both engines read `'2023-02-29'` as 2023-03-01 in `date()`, but Turso's `timediff()` keeps day 29 of February when it splits the dates into year/month/day, so the difference is off by the overflow, and the first query even prints `+-001-11-28`.

## Likely cause

SQLite's `isDate()` (`src/date.c`) clears `validYMD` when a lone `YYYY-MM-DD` has a day above 28 ("Make sure a YYYY-MM-DD is normalized. Example: 2023-02-31 -> 2023-03-03"), so `computeYMD_HMS()` recomputes the fields from the julian day. Turso's `exec_datetime_general` does the same (`if !has_modifier && p.valid_ymd && p.d > 28`), but `exec_timediff` (`core/functions/datetime.rs`) calls `parse_date_or_time` and then `compute_ymd_hms()` without it.

Tested on Turso main f8549ca2.
