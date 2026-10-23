Filed as #9884 (https://github.com/tursodatabase/turso/issues/9884) (line 1 corrected 2026-10-21).

# strftime() returns a value when the modifiers move the date out of range; date() and SQLite return NULL

## Reproduction

```sql
SELECT quote(strftime('%Y-%m-%d', 0, '-1 day'));
SELECT quote(strftime('%Y', 1, 'start of year'));
SELECT quote(strftime('%d', 5373484, '+1 day'));
SELECT quote(strftime('x', 0, '-1 day'));
SELECT quote(date(0, '-1 day')), quote(date(5373484, '+1 day'));
```

Turso (`tursodb -q -m list`, main f8549ca2):
```
'2000-01-01'
'-4713'
'01'
'x'
NULL|NULL
```

SQLite 3.50.4 (trunk f69fd5f5 gives the same):
```
NULL
NULL
NULL
NULL
NULL|NULL
```

## Expected

NULL, as for `date()`. SQLite checks the julian day after applying the modifiers (`isDate()` in `src/date.c`: `if( p->isError || !validJulianDay(p->iJD) ) return 1;`), and `strftime()` goes through the same check. A result before -4713-11-24 12:00 (julian day 0) or after 9999-12-31 is NULL. Turso returns a string, sometimes made from a default date (`'2000-01-01'` above), sometimes from the out-of-range one (`'-4713'`). In a `WHERE` clause this keeps rows SQLite drops.

## Likely cause

`exec_strftime` (`core/functions/datetime.rs`) only checks `p.is_error` after `p.compute_jd()`. `exec_datetime_general` in the same file also rejects `p.i_jd < 0 || p.i_jd > MAX_JD`; `exec_strftime` does not.

Related, closed: #5150 (strftime with a huge integer wrapped to year 2000).

Tested on Turso main f8549ca2.
