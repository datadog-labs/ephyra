Filed as #9921 (https://github.com/tursodatabase/turso/issues/9921) (line 1 corrected 2026-10-21).

# `±HH:MM` date modifier with hour 24 adds a day; SQLite adds nothing

```sql
SELECT datetime('2024-01-01 10:00', '+24:00');
SELECT unixepoch('2024-01-01', '+24:00');
SELECT datetime('2024-01-01 10:00', '+23:59');
```

Turso (main f8549ca2):
```
2024-01-02 10:00:00
1704153600
2024-01-02 09:59:00
```

SQLite 3.50.4 (and current trunk):
```
2024-01-01 10:00:00
1704067200
2024-01-02 09:59:00
```

A `±HH:MM[:SS.SSS]` modifier accepts hours up to 24. SQLite treats the value as a time of day and drops whole days from it (`parseModifier()` in date.c: `day = tx.iJD/86400000; tx.iJD -= day*86400000;`), so `'+24:00'` (and `'-24:00'`) leave the date unchanged. Turso adds the full 24 hours. Hours above 24 (`'+48:30'`) are rejected by both, and values below 24 agree.

Likely cause: "Case 2: HH:MM:SS Arithmetic" in `parse_modifier` (core/functions/datetime.rs) adds `h*3600000 + min*60000 + s*1000` without removing whole days.

Found with a differential test generator for SQLite databases and queries, comparing Turso with SQLite 3.50.4.
