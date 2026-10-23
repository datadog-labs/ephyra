Filed as #9969 (https://github.com/tursodatabase/turso/issues/9969) (line 1 corrected 2026-10-21).
Title: strftime() prints an overflowing day as is (2024-02-30); date() and SQLite normalize it to 2024-03-01

```sql
SELECT strftime('%F', '1970-02-31');
SELECT strftime('%Y-%m-%d', '2024-02-30');
SELECT strftime('%d', '2024-02-30');
SELECT strftime('%m', '2024-04-31');
SELECT strftime('%F %T', '2024-02-30 10:00');
SELECT date('2024-02-30');
```

Turso (`tursodb -q -m list`):

```
1970-02-31
2024-02-30
30
04
2024-02-30 10:00:00
2024-03-01
```

SQLite 3.50.4 (trunk is the same):

```
1970-03-03
2024-03-01
01
05
2024-03-01 10:00:00
2024-03-01
```

Expected: the SQLite output. A day past the end of the month moves into the next month (https://sqlite.org/lang_datefunc.html: "2023-02-31" is "2023-03-03"). date(), datetime() and strftime() with a modifier already do this. Only strftime() without a modifier prints the day from the input.

Likely cause: `exec_datetime_general` (core/functions/datetime.rs ~926) clears `valid_ymd` when there is no modifier and the day is above 28, so Y/M/D are computed again from the Julian day. `exec_strftime` (~1338) doesn't, so `compute_ymd_hms` keeps the parsed day. SQLite does this step in `isDate()`, which every date function shares.

This is the same missing step as #9886, where `timediff()` doesn't normalize a day above 28 either. It is not the out-of-range case of #9884, which is fixed.

Tested on Turso main abf885b2 (also on f8549ca2).
