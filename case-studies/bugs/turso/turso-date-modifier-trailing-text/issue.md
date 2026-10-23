Filed as #9837 (https://github.com/tursodatabase/turso/issues/9837) (line 1 corrected 2026-10-21).

Title: date/time "NNN units" and "±YYYY-MM-DD" modifiers accept trailing whitespace and trailing text that SQLite rejects

Reproduction (`tursodb -q -m list < repro.sql`):

```sql
SELECT quote(date('2024-01-01', '+1 day '));
SELECT quote(date('2024-01-01', '+1 day x'));
SELECT quote(date('2024-01-01', '+1 month '));
SELECT quote(date('2024-01-01', '+0001-00-01 '));
SELECT quote(date(0, '+1 day '));
```

Turso (7641fad2):
```
'2024-01-02'
'2024-01-02'
'2024-02-01'
'2025-01-02'
'-4713-11-25'
```

SQLite 3.50.4 (trunk f69fd5f5 gives the same):
```
NULL
NULL
NULL
NULL
NULL
```

Expected: NULL. In SQLite's parseModifier the unit has to match the rest of the string exactly ("day " and "day x" match no unit), and after a `±YYYY-MM-DD` part a space must be followed by a valid HH:MM time. A trailing tab or newline gives the same difference. `'+1  day'` (two spaces inside) is accepted by both engines, so this is only about text after the unit.

Likely cause: `core/functions/datetime.rs`, `parse_arithmetic_modifier`, starts with `z.trim()`. Its "NNN units" branch also uses `split_whitespace()` and never checks for tokens after the unit.

Tested on Turso main 7641fad2 (2026-10-13).
