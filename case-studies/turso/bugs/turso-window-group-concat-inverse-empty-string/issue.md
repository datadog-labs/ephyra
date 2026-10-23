Filed as #10352 (https://github.com/tursodatabase/turso/issues/10352) on 2026-10-21.
Title: group_concat() over a sliding window frame panics ("GroupConcat accumulator is Text after a non-NULL xStep") when the frame holds an empty string

```sql
CREATE TABLE t(a INTEGER PRIMARY KEY, b);
INSERT INTO t VALUES (1,'a'),(2,''),(3,NULL);
SELECT a, quote(group_concat(b) OVER (ORDER BY a ROWS BETWEEN CURRENT ROW AND 1 FOLLOWING)) FROM t;
```

Turso (`tursodb -q -m list :memory: < repro.sql`):
```
1|'a,'
2|NULL

thread 'main' panicked at core/vdbe/execute.rs:9240:17:
internal error: entered unreachable code: GroupConcat accumulator is Text after a non-NULL xStep
```

SQLite 3.50.4 (trunk gives the same):
```
1|'a,'
2|''
3|NULL
```

Expected: the SQLite output. Row 2's frame is `('', NULL)`, so the result is the empty string, not NULL. Row 3 should not panic.

Likely cause: in `inverse_agg_payload` (core/vdbe/execute.rs, the `GroupConcat` arm), removing `'a'` from the accumulator `"a,"` removes `len("a") + len(",") = 2` bytes, which is the whole text. The `remove_len >= accumulated_len` branch then sets the accumulator to NULL although one value (`''`) is still in the frame (`count` is 1). Row 2 therefore returns NULL, and the next xInverse (removing `''`) finds a NULL accumulator and reaches the `unreachable!`. Without the panic, the same thing gives a wrong NULL whenever only empty strings are left in the frame: with `('a'),(''),('')` and `ROWS BETWEEN CURRENT ROW AND 2 FOLLOWING`, row 3 is NULL in Turso and `''` in SQLite.

Tested on Turso main cbd5ac4e.
